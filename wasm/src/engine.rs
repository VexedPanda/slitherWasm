//! The HARD difficulty check (`CanSolveDifficulty.HARD.tsCanSolve` in TS): the fixpoint of the standard solvers
//! (standardSolverFactory), FaceColorDisconnectionSolver and the pattern rules (compiled clauses, see cansolve.rs and
//! compilePatternRules.ts), and whether it solves the puzzle.
//!
//! Every rule only adds information, and the fixpoint doesn't depend on the order rules are applied in (checked on
//! recorded generation checks, including with shuffled solver orders), so rules are re-evaluated from dirty queues until
//! nothing changes. Each rule follows its TS solver exactly, including some quirks noted below.
//!
//! State: edges (white/black/red), sectors (allowed counts of black edges among a corner's two edges), vertex states
//! (allowed pairs of black edges, or none), face states (allowed sets of black edges around a face), and face colors as
//! a union-find with parity (same/opposite) over the faces plus the outside.

// Many rules index several per-element arrays in parallel (edges, their colors, candidate bits), so index loops read
// more clearly here than zipped iterators
#![allow(clippy::needless_range_loop)]

use crate::cansolve::{Invalid, RuleSet};
use crate::topology::{NONE, Topology};

const NO_FACE_VALUE: u8 = 0xff;

const WHITE: u8 = 0;
const BLACK: u8 = 1;
const RED: u8 = 2;

// Sector bits: allowed numbers of black edges
const S0: u8 = 1;
const S1: u8 = 2;
const S2: u8 = 4;

type Result<T> = std::result::Result<T, Invalid>;

/// A color: union-find root and parity
type Color = (u32, u8);

// ---- face state candidates ----

/// The candidate sets of black edges for a face with n edges and a value (in TS's index order), as bitmasks over the
/// face's edges (bit i = face.edges[i])
fn face_candidates(n: usize, value: u8) -> Vec<u32> {
    if value == NO_FACE_VALUE {
        // getBinaryIndex: bit (n - 1 - i) of the index is edge i
        (0..1u32 << n)
            .map(|index| {
                (0..n)
                    .filter(|&i| index >> (n - 1 - i) & 1 == 1)
                    .map(|i| 1u32 << i)
                    .sum()
            })
            .collect()
    } else {
        // Combinations of `value` edges in lexicographic order of their sorted indices (index 0 = the first k edges)
        let k = value as usize;
        let mut result = Vec::new();
        let mut combination: Vec<usize> = (0..k).collect();
        if k > n {
            return result;
        }
        loop {
            result.push(combination.iter().map(|&i| 1u32 << i).sum());
            // next combination
            let mut i = k;
            loop {
                if i == 0 {
                    return result;
                }
                i -= 1;
                if combination[i] < n - k + i {
                    combination[i] += 1;
                    for j in i + 1..k {
                        combination[j] = combination[j - 1] + 1;
                    }
                    break;
                }
            }
        }
    }
}

struct Queue {
    items: Vec<u32>,
    queued: Vec<bool>,
}

impl Queue {
    fn new(size: usize, all: bool) -> Queue {
        Queue {
            items: if all {
                (0..size as u32).rev().collect()
            } else {
                Vec::new()
            },
            queued: vec![all; size],
        }
    }

    #[inline]
    fn push(&mut self, x: u32) {
        if !self.queued[x as usize] {
            self.queued[x as usize] = true;
            self.items.push(x);
        }
    }

    #[inline]
    fn pop(&mut self) -> Option<u32> {
        let x = self.items.pop()?;
        self.queued[x as usize] = false;
        Some(x)
    }
}

pub struct Engine<'a> {
    t: &'a Topology,
    rules: &'a RuleSet,
    face_values: &'a [u8],
    outside: u32,

    edge_state: Vec<u8>,
    sector_state: Vec<u8>,
    vertex_state: Vec<u128>,
    face_candidates: Vec<Vec<u32>>,
    face_state: Vec<Vec<u64>>,

    // Colors (node num_faces is the outside)
    color_parent: Vec<u32>,
    color_parity: Vec<u8>,
    color_members: Vec<Vec<u32>>,
    // Per root: member counts with parity 0 / 1 relative to the root
    color_counts: Vec<[u32; 2]>,

    // Black edge paths (see cansolve.rs)
    vblack: Vec<u8>,
    vwhite: Vec<u8>,
    fblack: Vec<u8>,
    fred: Vec<u8>,
    partner: Vec<u32>,
    closed_loops: u32,

    // Pattern clauses
    atom_true: Vec<bool>,
    atom_queue: Vec<u32>,
    remaining: Vec<u32>,
    alive: Vec<bool>,
    fired: Vec<bool>,
    color_recheck: Vec<u32>,

    // Dirty queues, one per kind of rule
    simple_vertex: Queue,
    simple_face: Queue,
    sector: Queue,
    vertex_from_edges: Queue,
    vertex_state_changed: Queue,
    edge_colors: Queue,
    face_parity: Queue,
    face_from_vertices: Queue,
    face_state_changed: Queue,
    changed_since_loop_scan: bool,
    colors_changed: bool,

    // Static per puzzle
    all_black_disallowed: Vec<bool>,
    null_face_parity_condition: Vec<bool>,
}

/// Whether the HARD rules solve the puzzle (face values one u8 per face, 0xff for none)
pub fn can_solve_hard(t: &Topology, rules: &RuleSet, face_values: &[u8]) -> Result<bool> {
    let section = profile::start();
    let mut engine = Engine::new(t, rules, face_values);
    profile::end(12, section);
    let section = profile::start();
    engine.init()?;
    profile::end(13, section);
    engine.run()?;
    Ok(engine.is_solved())
}

impl<'a> Engine<'a> {
    fn new(t: &'a Topology, rules: &'a RuleSet, face_values: &'a [u8]) -> Engine<'a> {
        let nf = t.num_faces;
        let face_candidates: Vec<Vec<u32>> = (0..nf)
            .map(|f| face_candidates(t.edges_of_face(f).len(), face_values[f]))
            .collect();
        let face_state = face_candidates
            .iter()
            .map(|candidates| {
                let mut bits = vec![0u64; candidates.len().div_ceil(64)];
                for i in 0..candidates.len() {
                    bits[i / 64] |= 1 << (i % 64);
                }
                bits
            })
            .collect();

        Engine {
            t,
            rules,
            face_values,
            outside: nf as u32,
            edge_state: vec![WHITE; t.num_edges],
            sector_state: vec![S0 | S1 | S2; t.num_half_edges],
            vertex_state: (0..t.num_vertices)
                .map(|v| {
                    let d = t.degree(v);
                    let size = d * (d - 1) / 2 + 1;
                    if size >= 128 { u128::MAX } else { (1u128 << size) - 1 }
                })
                .collect(),
            face_candidates,
            face_state,
            color_parent: (0..nf as u32 + 1).collect(),
            color_parity: vec![0; nf + 1],
            color_members: (0..nf as u32 + 1).map(|x| vec![x]).collect(),
            color_counts: vec![[1, 0]; nf + 1],
            vblack: vec![0; t.num_vertices],
            vwhite: (0..t.num_vertices).map(|v| t.degree(v) as u8).collect(),
            fblack: vec![0; nf],
            fred: vec![0; nf],
            partner: (0..t.num_vertices as u32).collect(),
            closed_loops: 0,
            atom_true: vec![false; rules.num_atoms],
            atom_queue: Vec::new(),
            remaining: vec![0; rules.clause_count()],
            alive: vec![false; rules.clause_count()],
            fired: vec![false; rules.clause_count()],
            color_recheck: Vec::new(),
            simple_vertex: Queue::new(t.num_vertices, true),
            simple_face: Queue::new(nf, false),
            sector: Queue::new(t.num_half_edges, true),
            vertex_from_edges: Queue::new(t.num_vertices, true),
            vertex_state_changed: Queue::new(t.num_vertices, false),
            edge_colors: Queue::new(t.num_edges, false),
            face_parity: Queue::new(nf, true),
            // VertexColorToFaceSolver starts with only the faces with values (see init)
            face_from_vertices: Queue::new(nf, false),
            face_state_changed: Queue::new(nf, false),
            changed_since_loop_scan: true,
            colors_changed: true,
            all_black_disallowed: vec![false; nf],
            null_face_parity_condition: vec![false; nf],
        }
    }

    // ---- static setup ----

    fn nonzero(&self, f: u32) -> bool {
        let value = self.face_values[f as usize];
        value != NO_FACE_VALUE && value != 0
    }

    fn init(&mut self) -> Result<()> {
        let t = self.t;
        let nf = t.num_faces;
        let nonzero_count = (0..nf as u32).filter(|&f| self.nonzero(f)).count();

        let valued_count = (0..nf).filter(|&f| self.face_values[f] != NO_FACE_VALUE).count();
        for f in 0..nf {
            let valued = self.face_values[f] != NO_FACE_VALUE;
            if valued {
                self.simple_face.push(f as u32);
                self.face_from_vertices.push(f as u32);
            }
            // hasNonzeroSeparateFace({F}): some other face has a value other than 0 or none
            let separate = nonzero_count > self.nonzero(f as u32) as usize;
            // fromVertexAndColorData: all edges black is disallowed if there's a separate nonzero face and some face with
            // a value has an edge that isn't one of this face's (any other face with a value does)
            let other_valued = valued_count - valued as usize;
            self.all_black_disallowed[f] = separate && other_valued > 0;
            // FaceColorParitySolver (faces without values): a separate nonzero face, and a face with a value (0
            // included) that isn't this face or a neighbor
            let valued_neighbors = t
                .neighbors_of_face(f)
                .iter()
                .filter(|&&g| self.face_values[g as usize] != NO_FACE_VALUE)
                .count();
            self.null_face_parity_condition[f] = separate && other_valued > valued_neighbors;
        }

        self.init_clauses(nonzero_count)?;
        self.static_double_minus_one_faces()?;
        self.static_sectors()?;
        Ok(())
    }

    fn init_clauses(&mut self, nonzero_count: usize) -> Result<()> {
        let rules = self.rules;
        let spanning_count = self.quick_spanning_face_count();
        for c in rules.candidate_clauses(self.face_values) {
            profile::count(0);
            let c = c as usize;
            let conditions = &rules.face_conditions
                [rules.face_condition_offsets[c] as usize..rules.face_condition_offsets[c + 1] as usize];
            if !conditions
                .iter()
                .all(|&(f, value)| self.face_values[f as usize] == value)
            {
                continue;
            }
            if rules.pattern_face_counts[c] as usize >= spanning_count {
                let faces = &rules.faces[rules.face_offsets[c] as usize..rules.face_offsets[c + 1] as usize];
                let inside = faces.iter().filter(|&&f| self.nonzero(f)).count();
                if nonzero_count == inside {
                    continue;
                }
            }
            profile::count(1);
            self.alive[c] = true;
            self.remaining[c] = rules.body(c).len() as u32;
            if self.remaining[c] == 0 {
                self.try_fire(c)?;
            }
        }
        Ok(())
    }

    fn quick_spanning_face_count(&self) -> usize {
        let t = self.t;
        let Some(start) = (0..t.num_faces as u32).find(|&f| self.nonzero(f)) else {
            return 0;
        };
        let mut visited = vec![false; t.num_faces];
        let mut in_next = vec![false; t.num_faces];
        let mut layer = vec![start];
        in_next[start as usize] = true;
        let mut count = 0;
        while !layer.is_empty() {
            count += 1;
            for &f in &layer {
                visited[f as usize] = true;
            }
            let mut next = Vec::new();
            for &f in &layer {
                for &v in t.vertices_of_face(f as usize) {
                    for &other in t.faces_of_vertex(v as usize) {
                        if !visited[other as usize] && !in_next[other as usize] {
                            in_next[other as usize] = true;
                            next.push(other);
                        }
                    }
                }
            }
            for &f in &layer {
                in_next[f as usize] = false;
            }
            layer = next;
        }
        count
    }

    // StaticDoubleMinusOneFacesSolver: two adjacent faces each one less than their edge count (e.g. two 3s in squares)
    fn static_double_minus_one_faces(&mut self) -> Result<()> {
        let t = self.t;
        let nf = t.num_faces;
        for m in 0..nf {
            let n_m = t.edges_of_face(m).len();
            if self.face_values[m] == NO_FACE_VALUE || self.face_values[m] as usize != n_m - 1 {
                continue;
            }
            for (i, &c) in t.edges_of_face(m).iter().enumerate() {
                let o = t.other_face(c, m as u32);
                if o == NONE {
                    continue;
                }
                let o = o as usize;
                let n_o = t.edges_of_face(o).len();
                if self.face_values[o] == NO_FACE_VALUE || self.face_values[o] as usize != n_o - 1 {
                    continue;
                }
                // A face with a positive value, not M, O, or a neighbor of either
                let near = |g: usize| {
                    g == m
                        || g == o
                        || t.neighbors_of_face(m).contains(&(g as u32))
                        || t.neighbors_of_face(o).contains(&(g as u32))
                };
                let has_third = (0..nf).any(|g| {
                    let value = self.face_values[g];
                    value != NO_FACE_VALUE && value > 0 && !near(g)
                });
                let separate = (0..nf).any(|g| g != m && g != o && self.nonzero(g as u32));
                if !has_third || !separate {
                    continue;
                }
                let h = t.half_edges_of_face(m)[i] as usize;
                let (a, b) = (t.he_start[h], t.he_end[h]);
                let mut black = vec![c];
                for f in [m, o] {
                    for &e in t.edges_of_face(f) {
                        let eh = t.edge_half_edge[e as usize] as usize;
                        let touches = [t.he_start[eh], t.he_end[eh]].iter().any(|&v| v == a || v == b);
                        if !touches {
                            black.push(e);
                        }
                    }
                }
                let mut red = Vec::new();
                for v in [a, b] {
                    for &e in t.edges_of_vertex(v as usize) {
                        if !t.edges_of_face(m).contains(&e) && !t.edges_of_face(o).contains(&e) {
                            red.push(e);
                        }
                    }
                }
                for e in black {
                    self.set_edge(e, BLACK)?;
                }
                for e in red {
                    self.set_edge(e, RED)?;
                }
            }
        }
        Ok(())
    }

    // StaticSectorSolver
    fn static_sectors(&mut self) -> Result<()> {
        let t = self.t;
        for f in 0..t.num_faces {
            let value = self.face_values[f];
            if value == NO_FACE_VALUE {
                continue;
            }
            let n = t.edges_of_face(f).len() as u8;
            let target = if value == 0 {
                S0
            } else if value == 1 {
                S0 | S1
            } else if value == n - 1 {
                S1 | S2
            } else if value == n {
                S2
            } else {
                continue;
            };
            for &h in t.half_edges_of_face(f) {
                self.restrict_sector(h, target)?;
            }
        }
        Ok(())
    }

    // ---- colors ----

    // Each node keeps its root and its parity relative to it (updated for the smaller class on each merge)
    #[inline]
    fn find(&self, x: u32) -> Color {
        (self.color_parent[x as usize], self.color_parity[x as usize])
    }

    /// The color of a face (NONE for the outside)
    #[inline]
    fn color(&self, f: u32) -> Color {
        self.find(if f == NONE { self.outside } else { f })
    }

    #[inline]
    fn node(&self, f: u32) -> u32 {
        if f == NONE { self.outside } else { f }
    }

    /// The opposite of a color, if it exists (some node has it, or it's the inside)
    #[inline]
    fn opposite(&self, c: Color) -> Option<Color> {
        let (root, parity) = c;
        let counts = self.color_counts[root as usize];
        if counts[(1 - parity) as usize] > 0 || self.find(self.outside).0 == root {
            Some((root, 1 - parity))
        } else {
            None
        }
    }

    #[inline]
    fn are_opposite(a: Color, b: Color) -> bool {
        a.0 == b.0 && a.1 != b.1
    }

    /// Makes two nodes' colors the same (or opposite)
    fn relate(&mut self, a: u32, b: u32, opposite: bool) -> Result<()> {
        let (ra, pa) = self.find(a);
        let (rb, pb) = self.find(b);
        let difference = opposite as u8;
        if ra == rb {
            return if pa ^ pb == difference { Ok(()) } else { Err(Invalid) };
        }
        // Merge the smaller class into the larger
        let (child, parent, flip) = if self.color_members[ra as usize].len() < self.color_members[rb as usize].len() {
            (ra, rb, pa ^ pb ^ difference)
        } else {
            (rb, ra, pa ^ pb ^ difference)
        };
        for &m in &self.color_members[child as usize] {
            self.color_parent[m as usize] = parent;
            self.color_parity[m as usize] ^= flip;
        }
        let child_counts = self.color_counts[child as usize];
        let parent_counts = &mut self.color_counts[parent as usize];
        if flip == 0 {
            parent_counts[0] += child_counts[0];
            parent_counts[1] += child_counts[1];
        } else {
            parent_counts[0] += child_counts[1];
            parent_counts[1] += child_counts[0];
        }
        let members = std::mem::take(&mut self.color_members[child as usize]);
        for &x in &members {
            self.color_changed(x);
        }
        self.color_members[parent as usize].extend(members);
        self.colors_changed = true;
        Ok(())
    }

    // Dirties what depends on a node's color relative to others
    fn color_changed(&mut self, x: u32) {
        let t = self.t;
        if x == self.outside {
            for &e in &t.boundary_edges {
                self.edge_colors.push(e);
            }
            for &f in &t.boundary_faces {
                self.face_parity.push(f);
                self.face_from_vertices.push(f);
            }
            for &v in &t.boundary_vertices {
                self.vertex_from_edges.push(v);
            }
        } else {
            let f = x as usize;
            for &e in t.edges_of_face(f) {
                self.edge_colors.push(e);
            }
            self.face_parity.push(x);
            self.face_from_vertices.push(x);
            for &g in t.neighbors_of_face(f) {
                self.face_parity.push(g);
                self.face_from_vertices.push(g);
            }
            for &v in t.vertices_of_face(f) {
                self.vertex_from_edges.push(v);
            }
        }
        for &c in self.rules.color_watches_of(x) {
            if self.alive[c as usize] && self.remaining[c as usize] == 0 && !self.fired[c as usize] {
                // (fired doubles as "queued for a recheck" until the recheck runs, see run)
                self.fired[c as usize] = true;
                self.color_recheck.push(c);
            }
        }
    }

    // ---- setting state ----

    fn set_edge(&mut self, e: u32, value: u8) -> Result<()> {
        let ei = e as usize;
        match self.edge_state[ei] {
            WHITE => {}
            current if current == value => return Ok(()),
            _ => return Err(Invalid),
        }
        let t = self.t;
        self.edge_state[ei] = value;
        let atom = 2 * e + (value == RED) as u32;
        self.atom_true[atom as usize] = true;
        self.atom_queue.push(atom);

        let h = t.edge_half_edge[ei] as usize;
        let r = t.he_reversed[h] as usize;
        let (a, b) = (t.he_start[h], t.he_end[h]);
        for v in [a, b] {
            self.vwhite[v as usize] -= 1;
            self.simple_vertex.push(v);
            self.vertex_from_edges.push(v);
        }
        for f in [t.he_face[h], t.he_face[r]] {
            if f != NONE {
                if value == BLACK {
                    self.fblack[f as usize] += 1;
                } else {
                    self.fred[f as usize] += 1;
                }
                self.simple_face.push(f);
                self.face_parity.push(f);
            }
        }
        for s in [h as u32, r as u32, t.he_previous[h], t.he_previous[r]] {
            self.sector.push(s);
        }

        if value == BLACK {
            let (ai, bi) = (a as usize, b as usize);
            if self.vblack[ai] >= 2 || self.vblack[bi] >= 2 {
                return Err(Invalid);
            }
            if self.vblack[ai] == 1 && self.vblack[bi] == 1 && self.partner[ai] == b {
                self.closed_loops += 1;
            } else {
                let pa = self.partner[ai];
                let pb = self.partner[bi];
                self.partner[pa as usize] = pb;
                self.partner[pb as usize] = pa;
            }
            self.vblack[ai] += 1;
            self.vblack[bi] += 1;
        }
        self.changed_since_loop_scan = true;

        // SafeEdgeToFaceColorSolver: black edges separate opposite colors, red edges join the same color
        let (fa, fb) = (self.node(t.he_face[h]), self.node(t.he_face[r]));
        self.relate(fa, fb, value == BLACK)
    }

    fn restrict_sector(&mut self, s: u32, allowed: u8) -> Result<()> {
        let old = self.sector_state[s as usize];
        let new = old & allowed;
        if new == old {
            return Ok(());
        }
        if new == 0 {
            return Err(Invalid);
        }
        self.sector_state[s as usize] = new;
        let base = 2 * self.t.num_edges as u32 + 3 * s;
        for (bit, flag) in [(S0, 0), (S1, 1), (S2, 2)] {
            if old & bit != 0 && new & bit == 0 {
                self.atom_true[(base + flag) as usize] = true;
                self.atom_queue.push(base + flag);
            }
        }
        self.sector.push(s);
        self.vertex_from_edges.push(self.t.he_end[s as usize]);
        Ok(())
    }

    fn restrict_vertex(&mut self, v: u32, allowed: u128) -> Result<()> {
        let old = self.vertex_state[v as usize];
        let new = old & allowed;
        if new == old {
            return Ok(());
        }
        if new == 0 {
            return Err(Invalid);
        }
        self.vertex_state[v as usize] = new;
        self.vertex_state_changed.push(v);
        for &f in self.t.faces_of_vertex(v as usize) {
            self.face_from_vertices.push(f);
        }
        Ok(())
    }

    fn restrict_face(&mut self, f: u32, allowed: &[u64]) -> Result<()> {
        let fi = f as usize;
        let mut changed = false;
        let mut any = false;
        for (word, &mask) in self.face_state[fi].iter_mut().zip(allowed) {
            let new = *word & mask;
            changed |= new != *word;
            any |= new != 0;
            *word = new;
        }
        if !any {
            return Err(Invalid);
        }
        if changed {
            self.face_state_changed.push(f);
        }
        Ok(())
    }

    // ---- pattern clauses ----

    // Fires a clause whose atoms hold, if its color conditions hold too
    fn try_fire(&mut self, c: usize) -> Result<()> {
        let rules = self.rules;
        for &(a, b, opposite) in rules.color_conditions_of(c) {
            let (ra, pa) = self.find(a);
            let (rb, pb) = self.find(b);
            if ra != rb {
                // Not yet (rechecked when either class changes)
                return Ok(());
            }
            if (pa != pb) != opposite {
                // Never (colors only merge)
                self.alive[c] = false;
                return Ok(());
            }
        }
        self.fired[c] = true;
        for &atom in rules.head(c) {
            self.set_atom(atom)?;
        }
        for &(a, b, opposite) in rules.color_outputs_of(c) {
            self.relate(a, b, opposite)?;
        }
        Ok(())
    }

    fn set_atom(&mut self, atom: u32) -> Result<()> {
        if self.atom_true[atom as usize] {
            return Ok(());
        }
        let num_edge_atoms = 2 * self.t.num_edges as u32;
        if atom < num_edge_atoms {
            self.set_edge(atom / 2, if atom & 1 == 0 { BLACK } else { RED })
        } else {
            let s = (atom - num_edge_atoms) / 3;
            let bit = [S0, S1, S2][((atom - num_edge_atoms) % 3) as usize];
            self.restrict_sector(s, !bit & 7)
        }
    }

    // ---- main loop ----

    fn run(&mut self) -> Result<()> {
        loop {
            let section = profile::start();
            let kind;
            if let Some(atom) = self.atom_queue.pop() {
                kind = 0;
                let rules = self.rules;
                for k in rules.watch_offsets[atom as usize] as usize..rules.watch_offsets[atom as usize + 1] as usize {
                    let c = rules.watches[k] as usize;
                    if self.alive[c] {
                        self.remaining[c] -= 1;
                        if self.remaining[c] == 0 {
                            self.try_fire(c)?;
                        }
                    }
                }
            } else if let Some(c) = self.color_recheck.pop() {
                kind = 1;
                self.fired[c as usize] = false;
                if self.alive[c as usize] {
                    self.try_fire(c as usize)?;
                }
            } else if let Some(v) = self.simple_vertex.pop() {
                kind = 2;
                self.simple_vertex_rules(v)?;
            } else if let Some(f) = self.simple_face.pop() {
                kind = 2;
                self.simple_face_rules(f)?;
            } else if let Some(s) = self.sector.pop() {
                kind = 3;
                self.sector_rules(s)?;
            } else if let Some(e) = self.edge_colors.pop() {
                kind = 4;
                self.simple_face_color(e)?;
            } else if let Some(v) = self.vertex_from_edges.pop() {
                kind = 5;
                self.vertex_from_edge_color_sector(v)?;
            } else if let Some(v) = self.vertex_state_changed.pop() {
                kind = 6;
                self.vertex_state_rules(v)?;
            } else if let Some(f) = self.face_parity.pop() {
                kind = 7;
                self.face_color_parity(f)?;
            } else if let Some(f) = self.face_from_vertices.pop() {
                kind = 8;
                self.face_from_vertex_and_color(f)?;
            } else if let Some(f) = self.face_state_changed.pop() {
                kind = 9;
                self.face_state_rules(f)?;
            } else if self.loop_rules()? {
                profile::end(10, section);
                continue;
            } else if self.colors_changed {
                kind = 11;
                self.colors_changed = false;
                self.face_color_disconnection()?;
            } else {
                return Ok(());
            }
            profile::end(kind, section);
        }
    }

    fn is_solved(&self) -> bool {
        self.closed_loops == 1 && (0..self.t.num_vertices).all(|v| self.vblack[v] != 1)
    }

    // SimpleVertexSolver
    fn simple_vertex_rules(&mut self, v: u32) -> Result<()> {
        let vi = v as usize;
        let black = self.vblack[vi];
        let white = self.vwhite[vi];
        if black > 2 || (black == 1 && white == 0) {
            return Err(Invalid);
        }
        if white == 0 {
            return Ok(());
        }
        let fill = match (black, white) {
            (2, _) => RED,
            (1, 1) => BLACK,
            (0, 1) => RED,
            _ => return Ok(()),
        };
        let t = self.t;
        for &e in t.edges_of_vertex(vi) {
            if self.edge_state[e as usize] == WHITE {
                self.set_edge(e, fill)?;
            }
        }
        Ok(())
    }

    // SimpleFaceSolver
    fn simple_face_rules(&mut self, f: u32) -> Result<()> {
        let fi = f as usize;
        let value = self.face_values[fi];
        let t = self.t;
        let n = t.edges_of_face(fi).len() as u8;
        let black = self.fblack[fi];
        let red = self.fred[fi];
        if black > value || red > n - value {
            return Err(Invalid);
        }
        if black + red == n {
            return Ok(());
        }
        let fill = if black == value {
            RED
        } else if red == n - value {
            BLACK
        } else {
            return Ok(());
        };
        for &e in t.edges_of_face(fi) {
            if self.edge_state[e as usize] == WHITE {
                self.set_edge(e, fill)?;
            }
        }
        Ok(())
    }

    // SafeEdgeToSectorSolver and SimpleSectorSolver
    fn sector_rules(&mut self, s: u32) -> Result<()> {
        let t = self.t;
        let si = s as usize;
        let a = t.he_edge[si];
        let b = t.he_edge[t.he_next[si] as usize];
        let states = [self.edge_state[a as usize], self.edge_state[b as usize]];
        let white = states.iter().filter(|&&x| x == WHITE).count();
        let black = states.iter().filter(|&&x| x == BLACK).count();
        let red = states.iter().filter(|&&x| x == RED).count();

        // SafeEdgeToSector
        match white {
            0 => self.restrict_sector(s, [S0, S1, S2][black])?,
            1 => {
                if black > 0 {
                    self.restrict_sector(s, S1 | S2)?;
                }
                if red > 0 {
                    self.restrict_sector(s, S0 | S1)?;
                }
            }
            _ => {}
        }

        // SimpleSector
        let state = self.sector_state[si];
        let allows = |n: usize| state & [S0, S1, S2][n] != 0;
        match white {
            0 => {
                if !allows(black) {
                    return Err(Invalid);
                }
            }
            1 => {
                let allows_red = allows(black);
                let allows_black = allows(black + 1);
                let white_edge = if states[0] == WHITE { a } else { b };
                match (allows_red, allows_black) {
                    (false, false) => return Err(Invalid),
                    (true, false) => self.set_edge(white_edge, RED)?,
                    (false, true) => self.set_edge(white_edge, BLACK)?,
                    _ => {}
                }
            }
            _ => {
                if state == S0 {
                    self.set_edge(a, RED)?;
                    self.set_edge(b, RED)?;
                } else if state == S2 {
                    self.set_edge(a, BLACK)?;
                    self.set_edge(b, BLACK)?;
                }
            }
        }
        Ok(())
    }

    // SimpleFaceColorSolver: a white edge between opposite colors is black, between the same color red
    fn simple_face_color(&mut self, e: u32) -> Result<()> {
        if self.edge_state[e as usize] != WHITE {
            return Ok(());
        }
        let [fa, fb] = self.t.faces_of_edge(e);
        let (a, b) = (self.color(fa), self.color(fb));
        if Self::are_opposite(a, b) {
            self.set_edge(e, BLACK)?;
        } else if a == b {
            self.set_edge(e, RED)?;
        }
        Ok(())
    }

    // ---- vertex states ----

    #[inline]
    fn pair_index(i: usize, j: usize, d: usize) -> usize {
        i * (2 * d - i - 1) / 2 + (j - i - 1)
    }

    // The colors of a vertex's face-ordered sectors: sectors[k] is between edges[k] and edges[k + 1]
    fn face_ordered_sector_colors(&self, v: usize) -> (Vec<u32>, Vec<Color>) {
        let t = self.t;
        let incoming = t.incoming_of_vertex(v);
        let d = incoming.len();
        let sectors: Vec<u32> = (0..d).map(|k| incoming[(k + 1) % d]).collect();
        let colors = sectors.iter().map(|&h| self.color(t.he_face[h as usize])).collect();
        (sectors, colors)
    }

    // A side of colors is consistent if it doesn't contain a color and its opposite
    fn has_opposites(colors: &[Color]) -> bool {
        colors.iter().any(|a| colors.iter().any(|b| Self::are_opposite(*a, *b)))
    }

    // SafeEdgeSectorColorToVertexSolver: VertexState.fromEdgeColorSectorData, intersected with the current state
    fn vertex_from_edge_color_sector(&mut self, v: u32) -> Result<()> {
        let t = self.t;
        let vi = v as usize;
        let edges = t.edges_of_vertex(vi);
        let d = edges.len();
        let size = d * (d - 1) / 2;
        let black: Vec<usize> = (0..d)
            .filter(|&k| self.edge_state[edges[k] as usize] == BLACK)
            .collect();
        let all_red = edges.iter().all(|&e| self.edge_state[e as usize] == RED);
        let mut allowed: u128 = 0;
        if black.len() > 2 {
            // none
        } else if black.len() == 2 {
            allowed = 1 << Self::pair_index(black[0], black[1], d);
        } else if all_red {
            allowed = 1 << size;
        } else {
            let (sectors, colors) = self.face_ordered_sector_colors(vi);
            for i in 0..d {
                for j in i + 1..d {
                    if self.edge_state[edges[i] as usize] == RED || self.edge_state[edges[j] as usize] == RED {
                        continue;
                    }
                    if black.len() == 1 && black[0] != i && black[0] != j {
                        continue;
                    }
                    let sectors_ok = sectors.iter().all(|&h| {
                        let (sa, sb) = (t.he_edge[h as usize], t.he_edge[t.he_next[h as usize] as usize]);
                        let count = [sa, sb].iter().filter(|&&e| e == edges[i] || e == edges[j]).count();
                        self.sector_state[h as usize] & [S0, S1, S2][count] != 0
                    });
                    if !sectors_ok {
                        continue;
                    }
                    let side_a = &colors[i..j];
                    let side_b: Vec<Color> = colors[j..].iter().chain(&colors[..i]).copied().collect();
                    if side_a.iter().any(|c| side_b.contains(c))
                        || Self::has_opposites(side_a)
                        || Self::has_opposites(&side_b)
                    {
                        continue;
                    }
                    allowed |= 1 << Self::pair_index(i, j, d);
                }
            }
            let empty_ok = black.is_empty()
                && sectors.iter().all(|&h| self.sector_state[h as usize] & S0 != 0)
                && !Self::has_opposites(&colors);
            if empty_ok {
                allowed |= 1 << size;
            }
        }
        self.restrict_vertex(v, allowed)
    }

    // The allowed pairs (i < j, as edge indices at the vertex) and whether empty is allowed
    fn allowed_pairs(&self, v: usize) -> (Vec<(usize, usize)>, bool) {
        let d = self.t.degree(v);
        let state = self.vertex_state[v];
        let mut pairs = Vec::new();
        for i in 0..d {
            for j in i + 1..d {
                if state >> Self::pair_index(i, j, d) & 1 == 1 {
                    pairs.push((i, j));
                }
            }
        }
        (pairs, state >> (d * (d - 1) / 2) & 1 == 1)
    }

    // VertexToEdgeSolver, VertexToSectorSolver and VertexToFaceColorSolver
    fn vertex_state_rules(&mut self, v: u32) -> Result<()> {
        let t = self.t;
        let vi = v as usize;
        let edges = t.edges_of_vertex(vi);
        let d = edges.len();
        let (pairs, empty) = self.allowed_pairs(vi);

        // VertexToEdge
        for k in 0..d {
            if self.edge_state[edges[k] as usize] != WHITE {
                continue;
            }
            let can_black = pairs.iter().any(|&(i, j)| i == k || j == k);
            let can_red = empty || pairs.iter().any(|&(i, j)| i != k && j != k);
            if can_black && !can_red {
                self.set_edge(edges[k], BLACK)?;
            } else if can_red && !can_black {
                self.set_edge(edges[k], RED)?;
            }
        }

        // VertexToSector
        for &h in t.incoming_of_vertex(vi) {
            let (sa, sb) = (t.he_edge[h as usize], t.he_edge[t.he_next[h as usize] as usize]);
            let mut allowed = if empty { S0 } else { 0 };
            for &(i, j) in &pairs {
                let count = [sa, sb].iter().filter(|&&e| e == edges[i] || e == edges[j]).count();
                allowed |= [S0, S1, S2][count];
            }
            self.restrict_sector(h, allowed)?;
        }

        // VertexToFaceColor
        let (_, colors) = self.face_ordered_sector_colors(vi);
        let mut distinct: Vec<Color> = Vec::new();
        for c in &colors {
            if !distinct.contains(c) {
                distinct.push(*c);
            }
        }
        if distinct.len() > 1 {
            let n = distinct.len();
            let index = |c: &Color| distinct.iter().position(|x| x == c).unwrap();
            let mut was_same = vec![false; n * n];
            let mut was_opposite = vec![false; n * n];
            for &(lo, hi) in &pairs {
                let side_a: Vec<usize> = colors[lo..hi].iter().map(index).collect();
                let side_b: Vec<usize> = colors[hi..].iter().chain(&colors[..lo]).map(index).collect();
                for side in [&side_a, &side_b] {
                    for &x in side.iter() {
                        for &y in side.iter() {
                            was_same[x * n + y] = true;
                        }
                    }
                }
                for &x in &side_a {
                    for &y in &side_b {
                        if x != y {
                            was_opposite[x * n + y] = true;
                            was_opposite[y * n + x] = true;
                        }
                    }
                }
            }
            if empty {
                was_same.iter_mut().for_each(|x| *x = true);
            }
            self.apply_color_deductions(&distinct, &was_same, &was_opposite)?;
        }
        Ok(())
    }

    // Make same (possible same, never opposite) or opposite (possible opposite, never same), for each pair of distinct
    // colors (VertexToFaceColorSolver, FaceToFaceColorSolver)
    fn apply_color_deductions(&mut self, distinct: &[Color], was_same: &[bool], was_opposite: &[bool]) -> Result<()> {
        let n = distinct.len();
        let nodes: Vec<u32> = distinct.iter().map(|&(root, _)| root).collect();
        let parities: Vec<u8> = distinct.iter().map(|&(_, parity)| parity).collect();
        for x in 0..n {
            for y in x + 1..n {
                let same = was_same[x * n + y];
                let opposite = was_opposite[x * n + y];
                // Relating roots with the colors' parities (a color (root, parity) is root's color flipped by parity)
                if same && !opposite {
                    self.relate(nodes[x], nodes[y], parities[x] != parities[y])?;
                } else if opposite && !same && !Self::are_opposite(distinct[x], distinct[y]) {
                    self.relate(nodes[x], nodes[y], parities[x] == parities[y])?;
                }
            }
        }
        Ok(())
    }

    // ---- face states ----

    // FaceState.fromVertexAndColorData, intersected with the current state (VertexColorToFaceSolver)
    fn face_from_vertex_and_color(&mut self, f: u32) -> Result<()> {
        let t = self.t;
        let fi = f as usize;
        let edges = t.edges_of_face(fi);
        let n = edges.len();
        let half_edges = t.half_edges_of_face(fi);
        let self_color = self.color(f);
        let across: Vec<Color> = edges.iter().map(|&e| self.color(t.other_face(e, f))).collect();

        // Binary combinations at each vertex: face vertex i is between edges i - 1 and i
        let mut vertex_combinations = Vec::with_capacity(n);
        for i in 0..n {
            let v = t.he_start[half_edges[i] as usize] as usize;
            let a = edges[(i + n - 1) % n];
            let b = edges[i];
            vertex_combinations.push(self.binary_combinations(v, a, b));
        }

        let candidates = &self.face_candidates[fi];
        let mut allowed = vec![0u64; candidates.len().div_ceil(64)];
        for (index, &mask) in candidates.iter().enumerate() {
            let has = |i: usize| mask >> i & 1 == 1;
            let vertices_ok = (0..n).all(|i| {
                let (none, both, a_only, b_only) = vertex_combinations[i];
                match (has((i + n - 1) % n), has(i)) {
                    (false, false) => none,
                    (true, true) => both,
                    (true, false) => a_only,
                    (false, true) => b_only,
                }
            });
            if !vertices_ok {
                continue;
            }
            let mut inside = vec![self_color];
            let mut outside = Vec::new();
            for i in 0..n {
                if has(i) {
                    outside.push(across[i]);
                } else {
                    inside.push(across[i]);
                }
            }
            if inside.iter().any(|c| outside.contains(c))
                || Self::has_opposites(&inside)
                || Self::has_opposites(&outside)
            {
                continue;
            }
            if mask.count_ones() as usize == n && self.all_black_disallowed[fi] {
                continue;
            }
            allowed[index / 64] |= 1 << (index % 64);
        }
        self.restrict_face(f, &allowed)
    }

    // getBinaryCombinationsAllowed(a, b) at a vertex: (none, both, a only, b only)
    fn binary_combinations(&self, v: usize, a: u32, b: u32) -> (bool, bool, bool, bool) {
        let edges = self.t.edges_of_vertex(v);
        let (pairs, empty) = self.allowed_pairs(v);
        let (mut none, mut both, mut a_only, mut b_only) = (empty, false, false, false);
        for (i, j) in pairs {
            let has_a = edges[i] == a || edges[j] == a;
            let has_b = edges[i] == b || edges[j] == b;
            match (has_a, has_b) {
                (true, true) => both = true,
                (true, false) => a_only = true,
                (false, true) => b_only = true,
                _ => none = true,
            }
        }
        (none, both, a_only, b_only)
    }

    fn allowed_face_masks(&self, f: usize) -> Vec<u32> {
        let candidates = &self.face_candidates[f];
        let state = &self.face_state[f];
        (0..candidates.len())
            .filter(|&i| state[i / 64] >> (i % 64) & 1 == 1)
            .map(|i| candidates[i])
            .collect()
    }

    // FaceToEdgeSolver, FaceToSectorSolver, FaceToFaceColorSolver and FaceToVertexSolver
    fn face_state_rules(&mut self, f: u32) -> Result<()> {
        let t = self.t;
        let fi = f as usize;
        let edges = t.edges_of_face(fi);
        let n = edges.len();
        let masks = self.allowed_face_masks(fi);
        let value = self.face_values[fi];

        // FaceToEdge. Quirk (FaceState.allowsEmpty): for a value k > 0, while the first k edges are an allowed
        // combination (candidate index 0), red counts as possible for every edge.
        let quirk = value != NO_FACE_VALUE && value > 0 && self.face_state[fi][0] & 1 == 1;
        for i in 0..n {
            if self.edge_state[edges[i] as usize] != WHITE {
                continue;
            }
            let can_black = masks.iter().any(|m| m >> i & 1 == 1);
            let can_red = quirk || masks.iter().any(|m| m >> i & 1 == 0);
            if can_black && !can_red {
                self.set_edge(edges[i], BLACK)?;
            } else if can_red && !can_black {
                self.set_edge(edges[i], RED)?;
            }
        }

        // FaceToSector: sector i (face.halfEdges[i]) is between edges i and i + 1
        for i in 0..n {
            let mut allowed = 0;
            for m in &masks {
                let count = (m >> i & 1) + (m >> ((i + 1) % n) & 1);
                allowed |= [S0, S1, S2][count as usize];
            }
            self.restrict_sector(t.half_edges_of_face(fi)[i], allowed)?;
        }

        // FaceToFaceColor
        let self_color = self.color(f);
        let across: Vec<Color> = edges.iter().map(|&e| self.color(t.other_face(e, f))).collect();
        let mut distinct = vec![self_color];
        for c in &across {
            if !distinct.contains(c) {
                distinct.push(*c);
            }
        }
        if distinct.len() > 1 {
            let k = distinct.len();
            let index = |c: &Color| distinct.iter().position(|x| x == c).unwrap();
            let mut was_same = vec![false; k * k];
            let mut was_opposite = vec![false; k * k];
            for m in &masks {
                let mut inside = vec![index(&self_color)];
                let mut outside = Vec::new();
                for i in 0..n {
                    if m >> i & 1 == 1 {
                        outside.push(index(&across[i]));
                    } else {
                        inside.push(index(&across[i]));
                    }
                }
                for side in [&inside, &outside] {
                    for &x in side.iter() {
                        for &y in side.iter() {
                            was_same[x * k + y] = true;
                        }
                    }
                }
                for &x in &inside {
                    for &y in &outside {
                        if x != y {
                            was_opposite[x * k + y] = true;
                            was_opposite[y * k + x] = true;
                        }
                    }
                }
            }
            self.apply_color_deductions(&distinct, &was_same, &was_opposite)?;
        }

        // FaceToVertex: face vertex i is between edges i - 1 and i
        for i in 0..n {
            let v = t.he_start[t.half_edges_of_face(fi)[i] as usize] as usize;
            let (ea, eb) = (edges[(i + n - 1) % n], edges[i]);
            let (ia, ib) = ((i + n - 1) % n, i);
            let (mut none, mut both, mut a_only, mut b_only) = (false, false, false, false);
            for m in &masks {
                match (m >> ia & 1 == 1, m >> ib & 1 == 1) {
                    (true, true) => both = true,
                    (true, false) => a_only = true,
                    (false, true) => b_only = true,
                    _ => none = true,
                }
            }
            let (old_none, old_both, old_a, old_b) = self.binary_combinations(v, ea, eb);
            let (none, both, a_only, b_only) = (none && old_none, both && old_both, a_only && old_a, b_only && old_b);
            if (none, both, a_only, b_only) == (old_none, old_both, old_a, old_b) {
                continue;
            }
            let vertex_edges = t.edges_of_vertex(v);
            let d = vertex_edges.len();
            let mut allowed: u128 = 0;
            for x in 0..d {
                for y in x + 1..d {
                    let has_a = vertex_edges[x] == ea || vertex_edges[y] == ea;
                    let has_b = vertex_edges[x] == eb || vertex_edges[y] == eb;
                    let flag = match (has_a, has_b) {
                        (true, true) => both,
                        (true, false) => a_only,
                        (false, true) => b_only,
                        _ => none,
                    };
                    if flag {
                        allowed |= 1 << Self::pair_index(x, y, d);
                    }
                }
            }
            if none {
                allowed |= 1 << (d * (d - 1) / 2);
            }
            self.restrict_vertex(v as u32, allowed)?;
        }
        Ok(())
    }

    // ---- FaceColorParitySolver ----

    fn face_color_parity(&mut self, f: u32) -> Result<()> {
        let t = self.t;
        let fi = f as usize;
        let edges = t.edges_of_face(fi);
        // Sides: white edges (in face order) with the color across them
        let sides: Vec<(u32, Color)> = edges
            .iter()
            .filter(|&&e| self.edge_state[e as usize] == WHITE)
            .map(|&e| (e, self.color(t.other_face(e, f))))
            .collect();
        if sides.is_empty() {
            return Ok(());
        }
        let value = self.face_values[fi];
        if value == NO_FACE_VALUE {
            let exterior = sides[0].1;
            let all_exterior = edges.iter().all(|&e| self.color(t.other_face(e, f)) == exterior);
            if all_exterior && self.null_face_parity_condition[fi] {
                for (e, _) in sides {
                    self.set_edge(e, RED)?;
                }
            }
            return Ok(());
        }
        let remaining = value as i32 - self.fblack[fi] as i32;
        match self.find_parity_action(remaining, &sides)? {
            None => Ok(()),
            Some(ParityAction::Edges(edges, state)) => {
                for e in edges {
                    self.set_edge(e, state)?;
                }
                self.face_parity.push(f);
                Ok(())
            }
            Some(ParityAction::Opposites(color, targets)) => {
                for target in targets {
                    self.relate(color.0, target.0, color.1 == target.1)?;
                }
                self.face_parity.push(f);
                Ok(())
            }
        }
    }

    fn find_parity_action(&self, black_needed: i32, sides: &[(u32, Color)]) -> Result<Option<ParityAction>> {
        // Group by color pair {c, opposite(c)}, in first-seen order
        struct Dual {
            main: Color,
            opposite: Option<Color>,
            main_sides: Vec<usize>,
            opposite_sides: Vec<usize>,
        }
        let mut duals: Vec<Dual> = Vec::new();
        for (index, &(_, color)) in sides.iter().enumerate() {
            if let Some(dual) = duals.iter_mut().find(|d| d.main == color || d.opposite == Some(color)) {
                if dual.main == color {
                    dual.main_sides.push(index);
                } else {
                    dual.opposite_sides.push(index);
                }
            } else {
                duals.push(Dual {
                    main: color,
                    opposite: self.opposite(color),
                    main_sides: vec![index],
                    opposite_sides: Vec::new(),
                });
            }
        }
        let size = |d: &Dual| d.main_sides.len().min(d.opposite_sides.len());
        // (stable sorts)
        duals.sort_by_key(|d| std::cmp::Reverse(size(d)));
        let mut singles: Vec<(Color, &Vec<usize>)> = Vec::new();
        for d in &duals {
            singles.push((d.main, &d.main_sides));
            if let Some(opposite) = d.opposite
                && !d.opposite_sides.is_empty()
            {
                singles.push((opposite, &d.opposite_sides));
            }
        }
        singles.sort_by_key(|(_, s)| std::cmp::Reverse(s.len()));

        let (largest_color, largest_sides) = singles[0];
        let m = largest_sides.len() as i32;
        let not_needed = sides.len() as i32 - black_needed;
        let is_red = m > black_needed;
        let is_black = m > not_needed;
        let is_balanced = m == black_needed.max(not_needed);
        if is_red && is_black {
            return Err(Invalid);
        }
        if is_red || is_black {
            let edges = largest_sides.iter().map(|&i| sides[i].0).collect();
            return Ok(Some(ParityAction::Edges(edges, if is_red { RED } else { BLACK })));
        }
        if is_balanced {
            let opposite = self.opposite(largest_color);
            let mut targets: Vec<Color> = Vec::new();
            for &(_, color) in sides {
                if color != largest_color && Some(color) != opposite && !targets.contains(&color) {
                    targets.push(color);
                }
            }
            if !targets.is_empty() {
                return Ok(Some(ParityAction::Opposites(largest_color, targets)));
            }
        }
        // Partial reduction: remove a balanced subset of a dual (it has as many black as red edges)
        for d in &duals {
            let k = size(d);
            if k < 1 {
                continue;
            }
            let removed: Vec<usize> = d.main_sides[..k]
                .iter()
                .chain(&d.opposite_sides[..k])
                .copied()
                .collect();
            let rest: Vec<(u32, Color)> = (0..sides.len())
                .filter(|i| !removed.contains(i))
                .map(|i| sides[i])
                .collect();
            if !rest.is_empty()
                && let Some(action) = self.find_parity_action(black_needed - k as i32, &rest)?
            {
                return Ok(Some(action));
            }
        }
        Ok(None)
    }

    // ---- SimpleLoopSolver ----

    fn loop_rules(&mut self) -> Result<bool> {
        if !self.changed_since_loop_scan {
            return Ok(false);
        }
        self.changed_since_loop_scan = false;
        let t = self.t;
        for a in 0..t.num_vertices as u32 {
            if self.vblack[a as usize] != 1 {
                continue;
            }
            let b = self.partner[a as usize];
            if b < a {
                continue;
            }
            for &candidate in t.edges_of_vertex(a as usize) {
                if self.edge_state[candidate as usize] != WHITE {
                    continue;
                }
                let mut path = vec![candidate];
                let mut current_edge = candidate;
                let mut current = t.other_vertex(candidate, a);
                while current != b {
                    let mut has_black = false;
                    let mut forced = NONE;
                    let mut white_count = 0;
                    for &e in t.edges_of_vertex(current as usize) {
                        if e == current_edge {
                            continue;
                        }
                        match self.edge_state[e as usize] {
                            BLACK => has_black = true,
                            WHITE => {
                                white_count += 1;
                                forced = e;
                            }
                            _ => {}
                        }
                    }
                    if white_count != 1 || has_black {
                        break;
                    }
                    current_edge = forced;
                    path.push(forced);
                    current = t.other_vertex(forced, current);
                }
                if current == b {
                    let fill = if self.solved_with_path(a, &path) { BLACK } else { RED };
                    for &e in &path {
                        self.set_edge(e, fill)?;
                    }
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn solved_with_path(&self, a: u32, path: &[u32]) -> bool {
        let t = self.t;
        let mut edges: Vec<u32> = path.to_vec();
        let mut vertex = a;
        let mut previous = NONE;
        loop {
            let mut next = NONE;
            for &e in t.edges_of_vertex(vertex as usize) {
                if e != previous && self.edge_state[e as usize] == BLACK {
                    next = e;
                    break;
                }
            }
            if next == NONE {
                break;
            }
            edges.push(next);
            previous = next;
            vertex = t.other_vertex(next, vertex);
        }
        let mut counts: Vec<(u32, u8)> = Vec::new();
        for &e in &edges {
            for f in t.faces_of_edge(e) {
                if f != NONE && self.face_values[f as usize] != NO_FACE_VALUE {
                    match counts.iter_mut().find(|(face, _)| *face == f) {
                        Some((_, count)) => *count += 1,
                        None => counts.push((f, 1)),
                    }
                }
            }
        }
        if counts.iter().any(|&(f, count)| self.face_values[f as usize] != count) {
            return false;
        }
        (0..t.num_faces as u32).all(|f| {
            let value = self.face_values[f as usize];
            value == NO_FACE_VALUE || value == 0 || counts.iter().any(|&(face, _)| face == f)
        })
    }

    // ---- FaceColorDisconnectionSolver ----

    // For each pair of colors meeting at a vertex (not opposite): if making them the same would leave a region
    // disconnected from the rest of the loop's requirements, they must be opposite. (Tests the half-edges of the faces
    // of both colors and their opposites, which finds everything any single TS run would; TS picks one of those two
    // pairs by random ids.)
    fn face_color_disconnection(&mut self) -> Result<()> {
        let t = self.t;
        let mut tested: Vec<(u32, u32, u8)> = Vec::new();
        let mut opposites: Vec<(Color, Color)> = Vec::new();
        for v in 0..t.num_vertices {
            let mut colors: Vec<Color> = Vec::new();
            for &h in t.incoming_of_vertex(v) {
                let c = self.color(t.he_face[h as usize]);
                if !colors.contains(&c) {
                    colors.push(c);
                }
            }
            for i in 0..colors.len() {
                for j in i + 1..colors.len() {
                    let (a, b) = (colors[i], colors[j]);
                    if Self::are_opposite(a, b) {
                        continue;
                    }
                    let key = (a.0.min(b.0), a.0.max(b.0), a.1 ^ b.1);
                    if tested.contains(&key) {
                        continue;
                    }
                    tested.push(key);
                    if self.disconnected_when_same(a, b) {
                        opposites.push((a, b));
                    }
                }
            }
        }
        for (a, b) in opposites {
            self.relate(a.0, b.0, a.1 == b.1)?;
        }
        Ok(())
    }

    fn disconnected_when_same(&self, a: Color, b: Color) -> bool {
        let t = self.t;
        // Colors in the hypothetical state where a and b are the same: b's class joins a's
        let delta = a.1 ^ b.1;
        let matched = |c: Color| -> Color { if c.0 == b.0 { (a.0, c.1 ^ delta) } else { c } };
        let half_edge_color = |h: u32| matched(self.color(t.he_face[h as usize]));

        // Half-edges of every edge of the faces of both classes (both parities)
        let mut to_check: Vec<u32> = Vec::new();
        let mut seen_half_edge = vec![false; t.num_half_edges];
        for root in [a.0, b.0] {
            for &x in &self.color_members[root as usize] {
                if x == self.outside {
                    continue;
                }
                for &e in t.edges_of_face(x as usize) {
                    let h = t.edge_half_edge[e as usize];
                    for hh in [h, t.he_reversed[h as usize]] {
                        if !seen_half_edge[hh as usize] {
                            seen_half_edge[hh as usize] = true;
                            to_check.push(hh);
                        }
                    }
                }
            }
        }

        let mut removed = vec![false; t.num_half_edges];
        for &h0 in &to_check {
            if removed[h0 as usize] {
                continue;
            }
            removed[h0 as usize] = true;
            let exterior = half_edge_color(t.he_reversed[h0 as usize]);
            let interior = half_edge_color(h0);
            if exterior == interior {
                continue;
            }
            // Trace the boundary with the exterior color on the right, turning as far left as possible
            let next = |mut h: u32| -> u32 {
                while half_edge_color(t.he_reversed[t.he_next[h as usize] as usize]) != exterior {
                    h = t.he_reversed[t.he_next[h as usize] as usize];
                }
                t.he_next[h as usize]
            };
            let mut boundary = vec![h0];
            let mut h = next(h0);
            let mut steps = 0;
            while h != h0 {
                boundary.push(h);
                removed[h as usize] = true;
                h = next(h);
                steps += 1;
                if steps > t.num_half_edges {
                    return false;
                }
            }
            let mut exterior_faces: Vec<u32> = Vec::new();
            let mut interior_faces: Vec<u32> = Vec::new();
            for &bh in &boundary {
                let ef = t.he_face[t.he_reversed[bh as usize] as usize];
                if !exterior_faces.contains(&ef) {
                    exterior_faces.push(ef);
                }
                let inf = t.he_face[bh as usize];
                if !interior_faces.contains(&inf) {
                    interior_faces.push(inf);
                }
            }
            if exterior_faces.len() == 1 {
                continue;
            }
            let cut: Vec<u32> = boundary.iter().map(|&bh| t.he_edge[bh as usize]).collect();
            if !self.is_component(&exterior_faces, false, &cut, exterior, &matched) {
                continue;
            }
            if self.is_component(&interior_faces, true, &cut, exterior, &matched) {
                return true;
            }
        }
        false
    }

    // Flood fill from faces (NONE is the outside) without crossing the cut: whether it reaches a face with a positive
    // value, or two opposite colors (counting the exterior color as seen)
    fn is_component(
        &self,
        initial: &[u32],
        call_on_initial: bool,
        cut: &[u32],
        exterior: Color,
        matched: &dyn Fn(Color) -> Color,
    ) -> bool {
        let t = self.t;
        let mut seen_colors: Vec<Color> = vec![exterior];
        let mut visited = vec![false; t.num_faces + 1];
        let index = |f: u32| if f == NONE { t.num_faces } else { f as usize };
        let check = |f: u32, seen_colors: &mut Vec<Color>| -> bool {
            if f != NONE {
                let value = self.face_values[f as usize];
                if value != NO_FACE_VALUE && value > 0 {
                    return true;
                }
            }
            let c = matched(self.color(f));
            if c != exterior && !seen_colors.contains(&c) {
                seen_colors.push(c);
            }
            let opposite = (c.0, 1 - c.1);
            seen_colors.contains(&opposite)
        };
        let mut stack: Vec<u32> = Vec::new();
        for &f in initial {
            if !visited[index(f)] {
                visited[index(f)] = true;
                if call_on_initial && check(f, &mut seen_colors) {
                    return true;
                }
                stack.push(f);
            }
        }
        while let Some(f) = stack.pop() {
            let mut visit = |g: u32, stack: &mut Vec<u32>, seen_colors: &mut Vec<Color>| -> bool {
                if !visited[index(g)] {
                    visited[index(g)] = true;
                    if check(g, seen_colors) {
                        return true;
                    }
                    stack.push(g);
                }
                false
            };
            if f == NONE {
                for &e in &t.boundary_edges {
                    if cut.contains(&e) {
                        continue;
                    }
                    let [fa, fb] = t.faces_of_edge(e);
                    let g = if fa == NONE { fb } else { fa };
                    if visit(g, &mut stack, &mut seen_colors) {
                        return true;
                    }
                }
            } else {
                for &e in t.edges_of_face(f as usize) {
                    if cut.contains(&e) {
                        continue;
                    }
                    let g = t.other_face(e, f);
                    if visit(g, &mut stack, &mut seen_colors) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

enum ParityAction {
    Edges(Vec<u32>, u8),
    Opposites(Color, Vec<Color>),
}

/// Time per kind of rule, in native builds with the "profile" feature (see examples/profile_hard.rs)
pub mod profile {
    #[cfg(feature = "profile")]
    pub static mut TOTALS: [f64; 14] = [0.0; 14];

    pub const NAMES: [&str; 14] = [
        "pattern atoms",
        "pattern color rechecks",
        "simple vertex/face",
        "sectors",
        "simple face color",
        "vertex from edges/sectors/colors",
        "vertex state rules",
        "face color parity",
        "face from vertices/colors",
        "face state rules",
        "simple loop",
        "color disconnection",
        "setup: allocation",
        "setup: static rules and initial clauses",
    ];

    #[cfg(feature = "profile")]
    pub static mut COUNTS: [u64; 4] = [0; 4];

    #[inline]
    pub fn count(_kind: usize) {
        #[cfg(feature = "profile")]
        unsafe {
            COUNTS[_kind] += 1
        }
    }

    #[cfg(feature = "profile")]
    #[inline]
    pub fn start() -> std::time::Instant {
        std::time::Instant::now()
    }

    #[cfg(feature = "profile")]
    #[inline]
    pub fn end(kind: usize, start: std::time::Instant) {
        unsafe { TOTALS[kind] += start.elapsed().as_secs_f64() }
    }

    #[cfg(not(feature = "profile"))]
    pub struct Section;

    #[cfg(not(feature = "profile"))]
    #[inline]
    pub fn start() -> Section {
        Section
    }

    #[cfg(not(feature = "profile"))]
    #[inline]
    pub fn end(_kind: usize, _start: Section) {}
}
