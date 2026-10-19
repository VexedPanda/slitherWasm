//! Difficulty checks (the EASY and MEDIUM `CanSolveDifficulty.canSolve` in TS): whether a set of deduction rules solves
//! a puzzle from its face values alone.
//!
//! The rules are those of the TS solvers used by those difficulties: SimpleVertexSolver, SimpleFaceSolver,
//! SimpleLoopSolver and BinaryPatternSolver (with the pattern rules compiled into clauses by compilePatternRules.ts).
//! Every rule only adds information, so the result is the same whatever order rules are applied in (checked against the
//! TS solver on recorded generation checks, including with shuffled rule orders). Here they're applied by forward
//! chaining: each clause counts its unsatisfied body atoms, and fires when that reaches zero.

use crate::solver::Board;

const NO_FACE_VALUE: u8 = 0xff;

// Edge state
const WHITE: u8 = 0;
const BLACK: u8 = 1;
const RED: u8 = 2;

pub struct RuleSet {
    num_atoms: usize,
    // Per clause, offsets into the arrays below (CSR)
    face_condition_offsets: Vec<u32>,
    face_conditions: Vec<(u32, u8)>,
    face_offsets: Vec<u32>,
    faces: Vec<u32>,
    body_offsets: Vec<u32>,
    head_offsets: Vec<u32>,
    atoms: Vec<u32>, // bodies and heads
    pattern_face_counts: Vec<u32>,
    // Atom => clauses with it in their body (CSR)
    watch_offsets: Vec<u32>,
    watches: Vec<u32>,
}

impl RuleSet {
    /// Parses the clauses from compilePatternRules.ts.
    pub fn new(num_edges: usize, num_sectors: usize, data: &[u32], clause_count: usize) -> RuleSet {
        let num_atoms = 2 * num_edges + 3 * num_sectors;
        let mut rules = RuleSet {
            num_atoms,
            face_condition_offsets: Vec::with_capacity(clause_count + 1),
            face_conditions: Vec::new(),
            face_offsets: Vec::with_capacity(clause_count + 1),
            faces: Vec::new(),
            body_offsets: Vec::with_capacity(clause_count + 1),
            head_offsets: Vec::with_capacity(clause_count + 1),
            atoms: Vec::new(),
            pattern_face_counts: Vec::with_capacity(clause_count),
            watch_offsets: Vec::new(),
            watches: Vec::new(),
        };
        let mut i = 0;
        for _ in 0..clause_count {
            rules.face_condition_offsets.push(rules.face_conditions.len() as u32);
            let count = data[i] as usize;
            i += 1;
            for _ in 0..count {
                rules.face_conditions.push((data[i], data[i + 1] as u8));
                i += 2;
            }

            rules.face_offsets.push(rules.faces.len() as u32);
            let count = data[i] as usize;
            i += 1;
            rules.faces.extend_from_slice(&data[i..i + count]);
            i += count;

            rules.body_offsets.push(rules.atoms.len() as u32);
            let count = data[i] as usize;
            i += 1;
            rules.atoms.extend_from_slice(&data[i..i + count]);
            i += count;

            rules.head_offsets.push(rules.atoms.len() as u32);
            let count = data[i] as usize;
            i += 1;
            rules.atoms.extend_from_slice(&data[i..i + count]);
            i += count;

            rules.pattern_face_counts.push(data[i]);
            i += 1;
        }
        rules.face_condition_offsets.push(rules.face_conditions.len() as u32);
        rules.face_offsets.push(rules.faces.len() as u32);
        rules.body_offsets.push(rules.atoms.len() as u32);
        rules.head_offsets.push(rules.atoms.len() as u32);

        // Watch lists
        let mut counts = vec![0u32; num_atoms + 1];
        for c in 0..clause_count {
            for &atom in rules.body(c) {
                counts[atom as usize + 1] += 1;
            }
        }
        for a in 0..num_atoms {
            counts[a + 1] += counts[a];
        }
        let mut fill = counts.clone();
        rules.watches = vec![0; counts[num_atoms] as usize];
        for c in 0..clause_count {
            for k in rules.body_offsets[c]..rules.head_offsets[c] {
                let atom = rules.atoms[k as usize] as usize;
                rules.watches[fill[atom] as usize] = c as u32;
                fill[atom] += 1;
            }
        }
        rules.watch_offsets = counts;
        rules
    }

    fn clause_count(&self) -> usize {
        self.pattern_face_counts.len()
    }

    #[inline]
    fn body(&self, c: usize) -> &[u32] {
        &self.atoms[self.body_offsets[c] as usize..self.head_offsets[c] as usize]
    }

    #[inline]
    fn head(&self, c: usize) -> &[u32] {
        &self.atoms[self.head_offsets[c] as usize..self.body_offsets[c + 1] as usize]
    }
}

/// A contradiction (the TS solvers throw InvalidStateError)
pub struct Invalid;

struct Checker<'a> {
    board: &'a Board,
    rules: &'a RuleSet,
    face_values: &'a [u8],
    edge_state: Vec<u8>,
    atom_true: Vec<bool>,
    atom_queue: Vec<u32>,
    remaining: Vec<u32>,
    alive: Vec<bool>,
    vblack: Vec<u8>,
    vwhite: Vec<u8>,
    fblack: Vec<u8>,
    fred: Vec<u8>,
    vertex_queue: Vec<u32>,
    in_vertex_queue: Vec<bool>,
    face_queue: Vec<u32>,
    in_face_queue: Vec<bool>,
    // Black edge paths: each path end's partner is its other end (see SimpleLoopSolver)
    partner: Vec<u32>,
    closed_loops: u32,
    // Whether any edge changed since the last SimpleLoopSolver scan
    changed_since_loop_scan: bool,
}

/// Whether the rules solve the puzzle with these face values (0xff for none)
pub fn can_solve(board: &Board, rules: &RuleSet, face_values: &[u8]) -> Result<bool, Invalid> {
    let mut checker = Checker {
        board,
        rules,
        face_values,
        edge_state: vec![WHITE; board.num_edges],
        atom_true: vec![false; rules.num_atoms],
        atom_queue: Vec::new(),
        remaining: vec![0; rules.clause_count()],
        alive: vec![false; rules.clause_count()],
        vblack: vec![0; board.num_vertices],
        vwhite: (0..board.num_vertices).map(|v| board.vertex_degree(v) as u8).collect(),
        fblack: vec![0; board.num_faces],
        fred: vec![0; board.num_faces],
        vertex_queue: (0..board.num_vertices as u32).collect(),
        in_vertex_queue: vec![true; board.num_vertices],
        face_queue: Vec::new(),
        in_face_queue: vec![false; board.num_faces],
        partner: (0..board.num_vertices as u32).collect(),
        closed_loops: 0,
        changed_since_loop_scan: true,
    };
    for (f, &value) in face_values.iter().enumerate() {
        if value != NO_FACE_VALUE {
            checker.face_queue.push(f as u32);
            checker.in_face_queue[f] = true;
        }
    }
    checker.init_clauses()?;
    checker.run()?;
    Ok(checker.is_solved())
}

impl Checker<'_> {
    // Which clauses can apply for this puzzle (face conditions, and BinaryPatternSolver's separate face check), firing
    // those with nothing left to wait for
    fn init_clauses(&mut self) -> Result<(), Invalid> {
        let board = self.board;
        let rules = self.rules;
        let nonzero = |f: u32| {
            let value = self.face_values[f as usize];
            value != NO_FACE_VALUE && value != 0
        };
        let nonzero_count = (0..board.num_faces as u32).filter(|&f| nonzero(f)).count();
        let spanning_count = self.quick_spanning_face_count();

        for c in 0..rules.clause_count() {
            let conditions = &rules.face_conditions
                [rules.face_condition_offsets[c] as usize..rules.face_condition_offsets[c + 1] as usize];
            if !conditions
                .iter()
                .all(|&(f, value)| self.face_values[f as usize] == value)
            {
                continue;
            }
            if rules.pattern_face_counts[c] as usize >= spanning_count {
                // Needs a nonzero face outside the rule's faces
                let faces = &rules.faces[rules.face_offsets[c] as usize..rules.face_offsets[c + 1] as usize];
                let inside = faces.iter().filter(|&&f| nonzero(f)).count();
                if nonzero_count == inside {
                    continue;
                }
            }
            self.alive[c] = true;
            self.remaining[c] = rules.body(c).len() as u32;
            if self.remaining[c] == 0 {
                self.fire(c)?;
            }
        }
        Ok(())
    }

    // getQuickBoardSpanningFaceCount: breadth-first layers (through shared vertices) from the first nonzero face
    fn quick_spanning_face_count(&self) -> usize {
        let board = self.board;
        let start = (0..board.num_faces).find(|&f| {
            let value = self.face_values[f];
            value != NO_FACE_VALUE && value != 0
        });
        let Some(start) = start else {
            return 0;
        };
        let mut visited = vec![false; board.num_faces];
        let mut in_next = vec![false; board.num_faces];
        let mut layer = vec![start as u32];
        in_next[start] = true;
        let mut count = 0;
        while !layer.is_empty() {
            count += 1;
            for &f in &layer {
                visited[f as usize] = true;
            }
            let mut next = Vec::new();
            for &f in &layer {
                for &v in board.face_vertices(f as usize) {
                    for &other in board.vertex_faces(v as usize) {
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

    fn fire(&mut self, c: usize) -> Result<(), Invalid> {
        let rules = self.rules;
        for &atom in rules.head(c) {
            self.set_atom(atom)?;
        }
        Ok(())
    }

    fn set_atom(&mut self, atom: u32) -> Result<(), Invalid> {
        if self.atom_true[atom as usize] {
            return Ok(());
        }
        let num_edge_atoms = 2 * self.board.num_edges as u32;
        if atom < num_edge_atoms {
            self.set_edge(atom / 2, if atom & 1 == 0 { BLACK } else { RED })
        } else {
            // Sector flags: the sector must keep at least one possible count
            let base = num_edge_atoms + 3 * ((atom - num_edge_atoms) / 3);
            let others = (base..base + 3)
                .filter(|&a| a != atom && self.atom_true[a as usize])
                .count();
            if others == 2 {
                return Err(Invalid);
            }
            self.atom_true[atom as usize] = true;
            self.atom_queue.push(atom);
            Ok(())
        }
    }

    fn set_edge(&mut self, e: u32, value: u8) -> Result<(), Invalid> {
        let ei = e as usize;
        match self.edge_state[ei] {
            WHITE => {}
            current if current == value => return Ok(()),
            _ => return Err(Invalid),
        }
        self.edge_state[ei] = value;
        let atom = 2 * e + (value == RED) as u32;
        self.atom_true[atom as usize] = true;
        self.atom_queue.push(atom);

        let [a, b] = self.board.edge_vertices[ei];
        for v in [a, b] {
            self.vwhite[v as usize] -= 1;
            self.enqueue_vertex(v);
        }
        for &f in &self.board.edge_faces[ei] {
            if f != u32::MAX {
                if value == BLACK {
                    self.fblack[f as usize] += 1;
                } else {
                    self.fred[f as usize] += 1;
                }
                self.enqueue_face(f);
            }
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
        Ok(())
    }

    #[inline]
    fn enqueue_vertex(&mut self, v: u32) {
        if !self.in_vertex_queue[v as usize] {
            self.in_vertex_queue[v as usize] = true;
            self.vertex_queue.push(v);
        }
    }

    #[inline]
    fn enqueue_face(&mut self, f: u32) {
        if self.face_values[f as usize] != NO_FACE_VALUE && !self.in_face_queue[f as usize] {
            self.in_face_queue[f as usize] = true;
            self.face_queue.push(f);
        }
    }

    fn run(&mut self) -> Result<(), Invalid> {
        loop {
            if let Some(atom) = self.atom_queue.pop() {
                let rules = self.rules;
                let start = rules.watch_offsets[atom as usize] as usize;
                let end = rules.watch_offsets[atom as usize + 1] as usize;
                for k in start..end {
                    let c = rules.watches[k] as usize;
                    if self.alive[c] {
                        self.remaining[c] -= 1;
                        if self.remaining[c] == 0 {
                            self.fire(c)?;
                        }
                    }
                }
            } else if let Some(v) = self.vertex_queue.pop() {
                self.in_vertex_queue[v as usize] = false;
                self.vertex_rules(v)?;
            } else if let Some(f) = self.face_queue.pop() {
                self.in_face_queue[f as usize] = false;
                self.face_rules(f)?;
            } else if !self.loop_rules()? {
                return Ok(());
            }
        }
    }

    // SimpleVertexSolver (all options on)
    fn vertex_rules(&mut self, v: u32) -> Result<(), Invalid> {
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
        let board = self.board;
        for &e in board.vertex_edges_of(vi) {
            if self.edge_state[e as usize] == WHITE {
                self.set_edge(e, fill)?;
            }
        }
        Ok(())
    }

    // SimpleFaceSolver (both options on)
    fn face_rules(&mut self, f: u32) -> Result<(), Invalid> {
        let fi = f as usize;
        let value = self.face_values[fi];
        let board = self.board;
        let n = board.face_edges_of(fi).len() as u8;
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
        for &e in board.face_edges_of(fi) {
            if self.edge_state[e as usize] == WHITE {
                self.set_edge(e, fill)?;
            }
        }
        Ok(())
    }

    // SimpleLoopSolver (solve to red and black): from one end of each open black path, a forced path of white edges
    // (each step through a vertex with no black edges and exactly one other white edge) back to its other end closes a
    // loop. That loop must be the solution: if the path's and loop's edges satisfy every face value, they're black,
    // otherwise the path is red. Returns whether anything changed.
    fn loop_rules(&mut self) -> Result<bool, Invalid> {
        // Only edge changes can enable this, so rescan only after some
        if !self.changed_since_loop_scan {
            return Ok(false);
        }
        self.changed_since_loop_scan = false;

        let board = self.board;
        for a in 0..board.num_vertices as u32 {
            if self.vblack[a as usize] != 1 {
                continue;
            }
            let b = self.partner[a as usize];
            if b < a {
                // Each path once (paths are symmetric: a forced path is forced in both directions)
                continue;
            }
            for &candidate in board.vertex_edges_of(a as usize) {
                if self.edge_state[candidate as usize] != WHITE {
                    continue;
                }
                let mut path = vec![candidate];
                let mut current_edge = candidate;
                let mut current = board.other_vertex(candidate, a);
                while current != b {
                    let mut has_black = false;
                    let mut forced = u32::MAX;
                    let mut white_count = 0;
                    for &e in board.vertex_edges_of(current as usize) {
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
                    current = board.other_vertex(forced, current);
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

    // Whether the black path from `a` plus the added edges satisfies every face value (isSolvedWithAddedEdges)
    fn solved_with_path(&self, a: u32, path: &[u32]) -> bool {
        let board = self.board;
        let mut edges: Vec<u32> = path.to_vec();
        // Walk the black path from a
        let mut vertex = a;
        let mut previous = u32::MAX;
        loop {
            let mut next = u32::MAX;
            for &e in board.vertex_edges_of(vertex as usize) {
                if e != previous && self.edge_state[e as usize] == BLACK {
                    next = e;
                    break;
                }
            }
            if next == u32::MAX {
                break;
            }
            edges.push(next);
            previous = next;
            vertex = board.other_vertex(next, vertex);
        }

        let mut counts: Vec<(u32, u8)> = Vec::new();
        for &e in &edges {
            for &f in &board.edge_faces[e as usize] {
                if f != u32::MAX && self.face_values[f as usize] != NO_FACE_VALUE {
                    match counts.iter_mut().find(|(face, _)| *face == f) {
                        Some((_, count)) => *count += 1,
                        None => counts.push((f, 1)),
                    }
                }
            }
        }
        // Touched faces must match exactly, and every other face with a value must be 0
        if counts.iter().any(|&(f, count)| self.face_values[f as usize] != count) {
            return false;
        }
        (0..board.num_faces as u32).all(|f| {
            let value = self.face_values[f as usize];
            value == NO_FACE_VALUE || value == 0 || counts.iter().any(|&(face, _)| face == f)
        })
    }

    // simpleRegionIsSolved: the black edges are exactly one closed loop
    fn is_solved(&self) -> bool {
        self.closed_loops == 1 && (0..self.board.num_vertices).all(|v| self.vblack[v] != 1)
    }
}
