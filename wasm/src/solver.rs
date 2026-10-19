//! Slitherlink solving with the CDCL SAT solver (sat.rs).
//!
//! A solution is a single closed loop of edges such that every face with a value has exactly that many loop edges, the
//! loop contains every given black edge and no given red edge, and the loop touches at least one face with a value.
//!
//! The local rules are clauses: each vertex has zero or two loop edges, and each face with a value has exactly that many.
//! Any model is then a set of disjoint loops. If exactly one loop touches a value (and contains the given black edges),
//! it's a solution. Every loop of the model is then excluded with a clause (it can't be the whole of a later model), and
//! the solver continues, until enough solutions are found or there are no more models. (Patterned after
//! https://www.dougandjean.com/slither/howitsolves.html, as was the original MiniSat-based solver.)

use crate::sat::{Sat, SolveStatus, lit};

pub const WHITE: u8 = 0;
pub const BLACK: u8 = 1;
pub const RED: u8 = 2;

pub const NO_VALUE: i8 = -1;

pub struct Board {
    pub num_vertices: usize,
    pub num_edges: usize,
    pub num_faces: usize,
    pub edge_vertices: Vec<[u32; 2]>,
    // CSR adjacency
    pub vertex_edge_offsets: Vec<u32>,
    pub vertex_edges: Vec<u32>,
    pub face_edge_offsets: Vec<u32>,
    pub face_edges: Vec<u32>,
    // Up to two faces per edge (u32::MAX when there is none, e.g. on the boundary)
    pub edge_faces: Vec<[u32; 2]>,
    // Vertices of each face, and faces of each vertex (CSR)
    face_vertex_offsets: Vec<u32>,
    face_vertices: Vec<u32>,
    vertex_face_offsets: Vec<u32>,
    vertex_faces: Vec<u32>,
}

impl Board {
    pub fn new(num_vertices: usize, edge_vertices: &[u32], face_edge_offsets: &[u32], face_edges: &[u32]) -> Board {
        let num_edges = edge_vertices.len() / 2;
        let num_faces = face_edge_offsets.len() - 1;

        let mut degree = vec![0u32; num_vertices];
        let mut ev = Vec::with_capacity(num_edges);
        for e in 0..num_edges {
            let a = edge_vertices[2 * e];
            let b = edge_vertices[2 * e + 1];
            degree[a as usize] += 1;
            degree[b as usize] += 1;
            ev.push([a, b]);
        }
        let mut vertex_edge_offsets = vec![0u32; num_vertices + 1];
        for v in 0..num_vertices {
            vertex_edge_offsets[v + 1] = vertex_edge_offsets[v] + degree[v];
        }
        let mut fill = vertex_edge_offsets.clone();
        let mut vertex_edges = vec![0u32; vertex_edge_offsets[num_vertices] as usize];
        for (e, [a, b]) in ev.iter().enumerate() {
            vertex_edges[fill[*a as usize] as usize] = e as u32;
            fill[*a as usize] += 1;
            vertex_edges[fill[*b as usize] as usize] = e as u32;
            fill[*b as usize] += 1;
        }

        let mut edge_faces = vec![[u32::MAX; 2]; num_edges];
        for f in 0..num_faces {
            for i in face_edge_offsets[f]..face_edge_offsets[f + 1] {
                let e = face_edges[i as usize] as usize;
                if edge_faces[e][0] == u32::MAX {
                    edge_faces[e][0] = f as u32;
                } else {
                    edge_faces[e][1] = f as u32;
                }
            }
        }

        let mut face_vertex_offsets = vec![0u32];
        let mut face_vertices: Vec<u32> = Vec::new();
        let mut vertex_face_lists: Vec<Vec<u32>> = vec![Vec::new(); num_vertices];
        for f in 0..num_faces {
            let start = face_vertices.len();
            for i in face_edge_offsets[f]..face_edge_offsets[f + 1] {
                for v in ev[face_edges[i as usize] as usize] {
                    if !face_vertices[start..].contains(&v) {
                        face_vertices.push(v);
                        vertex_face_lists[v as usize].push(f as u32);
                    }
                }
            }
            face_vertex_offsets.push(face_vertices.len() as u32);
        }
        let mut vertex_face_offsets = vec![0u32];
        let mut vertex_faces: Vec<u32> = Vec::new();
        for list in &vertex_face_lists {
            vertex_faces.extend_from_slice(list);
            vertex_face_offsets.push(vertex_faces.len() as u32);
        }

        Board {
            num_vertices,
            num_edges,
            num_faces,
            edge_vertices: ev,
            vertex_edge_offsets,
            vertex_edges,
            face_edge_offsets: face_edge_offsets.to_vec(),
            face_edges: face_edges.to_vec(),
            edge_faces,
            face_vertex_offsets,
            face_vertices,
            vertex_face_offsets,
            vertex_faces,
        }
    }

    pub fn face_edge_count(&self, f: usize) -> usize {
        (self.face_edge_offsets[f + 1] - self.face_edge_offsets[f]) as usize
    }

    pub fn vertex_degree(&self, v: usize) -> usize {
        (self.vertex_edge_offsets[v + 1] - self.vertex_edge_offsets[v]) as usize
    }

    #[inline]
    pub fn vertex_edges_of(&self, v: usize) -> &[u32] {
        self.vertex_edges(v)
    }

    #[inline]
    pub fn face_edges_of(&self, f: usize) -> &[u32] {
        self.face_edges(f)
    }

    #[inline]
    pub fn other_vertex(&self, e: u32, v: u32) -> u32 {
        let [a, b] = self.edge_vertices[e as usize];
        if a == v { b } else { a }
    }

    #[inline]
    pub fn face_vertices(&self, f: usize) -> &[u32] {
        &self.face_vertices[self.face_vertex_offsets[f] as usize..self.face_vertex_offsets[f + 1] as usize]
    }

    #[inline]
    pub fn vertex_faces(&self, v: usize) -> &[u32] {
        &self.vertex_faces[self.vertex_face_offsets[v] as usize..self.vertex_face_offsets[v + 1] as usize]
    }

    #[inline]
    fn vertex_edges(&self, v: usize) -> &[u32] {
        &self.vertex_edges[self.vertex_edge_offsets[v] as usize..self.vertex_edge_offsets[v + 1] as usize]
    }

    #[inline]
    fn face_edges(&self, f: usize) -> &[u32] {
        &self.face_edges[self.face_edge_offsets[f] as usize..self.face_edge_offsets[f + 1] as usize]
    }
}

pub enum SolveResult {
    // Solutions found (each a loop's edges, in loop order), stopping at the requested maximum
    Solutions(Vec<Vec<u32>>),
    // Gave up (too many iterations or conflicts)
    BudgetExceeded,
}

// Calls back with every subset of `items` of size `size`
fn for_each_combination(items: &[u32], size: usize, callback: &mut impl FnMut(&[u32])) {
    fn recurse(items: &[u32], size: usize, start: usize, current: &mut Vec<u32>, callback: &mut impl FnMut(&[u32])) {
        if current.len() == size {
            callback(current);
            return;
        }
        for i in start..items.len() {
            if items.len() - i < size - current.len() {
                break;
            }
            current.push(items[i]);
            recurse(items, size, i + 1, current, callback);
            current.pop();
        }
    }
    if size <= items.len() {
        recurse(items, size, 0, &mut Vec::with_capacity(size), callback);
    }
}

#[inline]
fn pos(e: u32) -> u32 {
    lit(e, false)
}

#[inline]
fn neg(e: u32) -> u32 {
    lit(e, true)
}

/// Finds up to `max_solutions` solutions, giving up after `max_iterations` models or `conflict_budget` conflicts.
/// Returns the solutions and the number of conflicts used.
pub fn solve(
    board: &Board,
    face_values: &[i8],
    edge_states: &[u8],
    max_solutions: usize,
    max_iterations: u64,
    conflict_budget: u64,
) -> (SolveResult, u64) {
    solve_excluding(
        board,
        face_values,
        edge_states,
        max_solutions,
        max_iterations,
        conflict_budget,
        None,
    )
}

/// Like solve, but only finds solutions other than a known one (e.g. to check whether it's unique): any other single
/// loop uses an edge outside it and leaves out one of its edges (one loop can't contain another), which also rules out
/// models that are the known solution plus another loop. The search also tries the known solution's opposite first.
pub fn solve_excluding(
    board: &Board,
    face_values: &[i8],
    edge_states: &[u8],
    max_solutions: usize,
    max_iterations: u64,
    conflict_budget: u64,
    known_solution: Option<&[u32]>,
) -> (SolveResult, u64) {
    if !face_values.iter().any(|&v| v > 0) {
        // Every loop would either break a 0 or touch no value
        return (SolveResult::Solutions(Vec::new()), 0);
    }

    let mut sat = Sat::new(board.num_edges);
    let mut ok = true;
    let mut clause: Vec<u32> = Vec::new();

    // Given edges
    for e in 0..board.num_edges as u32 {
        match edge_states[e as usize] {
            BLACK => ok &= sat.add_clause(&[pos(e)]),
            RED => ok &= sat.add_clause(&[neg(e)]),
            _ => {}
        }
    }

    // At least one edge
    clause.extend((0..board.num_edges as u32).map(pos));
    ok &= sat.add_clause(&clause);

    // Zero or two edges at each vertex: no three, and each edge implies another
    for v in 0..board.num_vertices {
        let edges = board.vertex_edges(v);
        for_each_combination(edges, 3, &mut |combination| {
            let negated: Vec<u32> = combination.iter().map(|&e| neg(e)).collect();
            ok &= sat.add_clause(&negated);
        });
        for &e in edges {
            clause.clear();
            clause.push(neg(e));
            clause.extend(edges.iter().filter(|&&other| other != e).map(|&other| pos(other)));
            ok &= sat.add_clause(&clause);
        }
    }

    // Exactly the value's number of edges around each face with a value
    for (f, &value) in face_values.iter().enumerate().take(board.num_faces) {
        if value == NO_VALUE {
            continue;
        }
        let value = value as usize;
        let edges = board.face_edges(f);
        if value > edges.len() {
            ok = false;
            continue;
        }
        // At least `value`: any (n - value + 1) edges include one in the loop
        if value > 0 {
            for_each_combination(edges, edges.len() - value + 1, &mut |combination| {
                let positive: Vec<u32> = combination.iter().map(|&e| pos(e)).collect();
                ok &= sat.add_clause(&positive);
            });
        }
        // At most `value`: no (value + 1) edges are all in the loop
        for_each_combination(edges, value + 1, &mut |combination| {
            let negated: Vec<u32> = combination.iter().map(|&e| neg(e)).collect();
            ok &= sat.add_clause(&negated);
        });
    }

    if let Some(known) = known_solution {
        let mut in_known = vec![false; board.num_edges];
        for &e in known {
            in_known[e as usize] = true;
        }
        clause.clear();
        clause.extend((0..board.num_edges as u32).filter(|&e| !in_known[e as usize]).map(pos));
        ok &= sat.add_clause(&clause);
        clause.clear();
        clause.extend(known.iter().map(|&e| neg(e)));
        ok &= sat.add_clause(&clause);
        for e in 0..board.num_edges as u32 {
            sat.set_preferred_value(e, in_known[e as usize]);
        }
    }

    let mut solutions: Vec<Vec<u32>> = Vec::new();
    if !ok {
        return (SolveResult::Solutions(solutions), 0);
    }

    let given_black: Vec<u32> = (0..board.num_edges as u32)
        .filter(|&e| edge_states[e as usize] == BLACK)
        .collect();
    let touches_value = |e: u32| {
        board.edge_faces[e as usize]
            .iter()
            .any(|&f| f != u32::MAX && face_values[f as usize] != NO_VALUE)
    };

    crate::generator::profile::count(3, 0);
    let _solve_timer = crate::generator::profile::Timer::new(4);
    let mut in_model = vec![false; board.num_edges];
    let mut visited = vec![false; board.num_edges];
    let mut iterations = 0u64;

    loop {
        if sat.conflicts >= conflict_budget {
            return (SolveResult::BudgetExceeded, sat.conflicts);
        }
        match sat.solve(conflict_budget - sat.conflicts) {
            SolveStatus::Unsat => break,
            SolveStatus::Unknown => return (SolveResult::BudgetExceeded, sat.conflicts),
            SolveStatus::Sat => {}
        }

        // Split the model into loops
        let mut model_edges: Vec<u32> = Vec::new();
        for e in 0..board.num_edges as u32 {
            let black = sat.model_value(e);
            in_model[e as usize] = black;
            visited[e as usize] = false;
            if black {
                model_edges.push(e);
            }
        }
        sat.reset_to_root();

        let mut loops: Vec<Vec<u32>> = Vec::new();
        let mut touching_loops = 0;
        let mut touching_loop_index = 0;
        for &start in &model_edges {
            if visited[start as usize] {
                continue;
            }
            let mut current_loop = Vec::new();
            let mut touching = false;
            let mut edge = start;
            let mut vertex = board.edge_vertices[start as usize][1];
            loop {
                visited[edge as usize] = true;
                current_loop.push(edge);
                touching |= touches_value(edge);
                let mut next = u32::MAX;
                for &e in board.vertex_edges(vertex as usize) {
                    if in_model[e as usize] && !visited[e as usize] {
                        next = e;
                        break;
                    }
                }
                if next == u32::MAX {
                    break;
                }
                let [a, b] = board.edge_vertices[next as usize];
                vertex = if a == vertex { b } else { a };
                edge = next;
            }
            if touching {
                touching_loops += 1;
                touching_loop_index = loops.len();
            }
            loops.push(current_loop);
        }

        if touching_loops == 0 {
            // Then every value is 0 (any loop near a positive value would touch it)
            break;
        }
        if touching_loops == 1 {
            let candidate = &loops[touching_loop_index];
            if given_black.iter().all(|e| candidate.contains(e)) {
                solutions.push(candidate.clone());
                if solutions.len() >= max_solutions {
                    break;
                }
            }
        }

        // Exclude these loops
        let mut satisfiable = true;
        for current_loop in &loops {
            clause.clear();
            clause.extend(current_loop.iter().map(|&e| neg(e)));
            satisfiable &= sat.add_clause(&clause);
        }
        if !satisfiable {
            break;
        }

        iterations += 1;
        crate::generator::profile::count(5, 0);
        if iterations > max_iterations {
            return (SolveResult::BudgetExceeded, sat.conflicts);
        }
    }

    (SolveResult::Solutions(solutions), sat.conflicts)
}

/// Checks whether a known solution stays unique as values are removed (puzzle minimization), with one SAT solver
/// throughout, so what it learns carries over between checks:
///
/// - Each face's value constraint is guarded by a selector variable, assumed true while the face has its value
/// - Any other solution must use an edge outside the known one and leave out one of its edges (see solve_excluding)
/// - Each loop excluded is guarded by the selector of a value the loop alone breaks (preferring values that are kept),
///   so the exclusion stays valid in later checks while that value remains
pub struct Uniqueness {
    sat: Sat,
    values: Vec<u8>,
    selectors: Vec<u32>,
}

const NO_SELECTOR: u32 = u32::MAX;

// Conflicts a uniqueness check may take before giving up
const UNIQUENESS_CONFLICT_BUDGET: u64 = 20_000;

impl Uniqueness {
    /// From the full puzzle (0xff for faces without values) and its solution
    pub fn new(board: &Board, values: &[u8], solution: &[u32]) -> Uniqueness {
        let mut num_vars = board.num_edges as u32;
        let selectors: Vec<u32> = values
            .iter()
            .map(|&v| {
                if v == 0xff {
                    NO_SELECTOR
                } else {
                    num_vars += 1;
                    num_vars - 1
                }
            })
            .collect();
        let mut sat = Sat::new(num_vars as usize);
        let mut clause: Vec<u32> = Vec::new();

        clause.extend((0..board.num_edges as u32).map(pos));
        sat.add_clause(&clause);
        for v in 0..board.num_vertices {
            let edges = board.vertex_edges(v);
            for_each_combination(edges, 3, &mut |combination| {
                let negated: Vec<u32> = combination.iter().map(|&e| neg(e)).collect();
                sat.add_clause(&negated);
            });
            for &e in edges {
                clause.clear();
                clause.push(neg(e));
                clause.extend(edges.iter().filter(|&&other| other != e).map(|&other| pos(other)));
                sat.add_clause(&clause);
            }
        }
        for f in 0..board.num_faces {
            if selectors[f] == NO_SELECTOR {
                continue;
            }
            let guard = lit(selectors[f], true);
            let value = values[f] as usize;
            let edges = board.face_edges(f);
            if value > 0 {
                for_each_combination(edges, edges.len() - value + 1, &mut |combination| {
                    let mut guarded = vec![guard];
                    guarded.extend(combination.iter().map(|&e| pos(e)));
                    sat.add_clause(&guarded);
                });
            }
            for_each_combination(edges, value + 1, &mut |combination| {
                let mut guarded = vec![guard];
                guarded.extend(combination.iter().map(|&e| neg(e)));
                sat.add_clause(&guarded);
            });
        }

        // Not the known solution (nor it plus another loop)
        let mut in_known = vec![false; board.num_edges];
        for &e in solution {
            in_known[e as usize] = true;
        }
        clause.clear();
        clause.extend((0..board.num_edges as u32).filter(|&e| !in_known[e as usize]).map(pos));
        sat.add_clause(&clause);
        clause.clear();
        clause.extend(solution.iter().map(|&e| neg(e)));
        sat.add_clause(&clause);
        for e in 0..board.num_edges as u32 {
            sat.set_preferred_value(e, in_known[e as usize]);
        }

        Uniqueness {
            sat,
            values: values.to_vec(),
            selectors,
        }
    }

    /// Whether the known solution is the only one with the values of the active faces. `kept` faces will keep their
    /// values (so exclusions guarded by them stay useful). None if the solver gave up.
    pub fn is_unique(&mut self, board: &Board, active: &[bool], kept: &[bool], max_iterations: u64) -> Option<bool> {
        #[cfg(feature = "profile")]
        let start = std::time::Instant::now();
        let result = self.is_unique_inner(board, active, kept, max_iterations);
        #[cfg(feature = "profile")]
        if start.elapsed().as_millis() > 50 {
            eprintln!(
                "  uniqueness: {} values, {:?} in {} ms",
                active.iter().filter(|&&a| a).count(),
                result,
                start.elapsed().as_millis()
            );
        }
        result
    }

    fn is_unique_inner(&mut self, board: &Board, active: &[bool], kept: &[bool], max_iterations: u64) -> Option<bool> {
        // Proofs that need very many conflicts are given up on (the caller then treats the puzzle as maybe not unique)
        let budget_end = self.sat.conflicts + UNIQUENESS_CONFLICT_BUDGET;
        let assumptions: Vec<u32> = (0..board.num_faces)
            .filter(|&f| active[f] && self.selectors[f] != NO_SELECTOR)
            .map(|f| lit(self.selectors[f], false))
            .collect();
        let touches_value = |e: u32| {
            board.edge_faces[e as usize]
                .iter()
                .any(|&f| f != u32::MAX && active[f as usize])
        };
        let mut in_model = vec![false; board.num_edges];
        let mut visited = vec![false; board.num_edges];
        let mut clause: Vec<u32> = Vec::new();
        for _ in 0..max_iterations {
            match self
                .sat
                .solve_assuming(&assumptions, budget_end.saturating_sub(self.sat.conflicts))
            {
                SolveStatus::Unsat => return Some(true),
                SolveStatus::Unknown => return None,
                SolveStatus::Sat => {}
            }
            let mut model_edges = Vec::new();
            for e in 0..board.num_edges as u32 {
                let black = self.sat.model_value(e);
                in_model[e as usize] = black;
                visited[e as usize] = false;
                if black {
                    model_edges.push(e);
                }
            }
            self.sat.reset_to_root();

            let mut loops: Vec<Vec<u32>> = Vec::new();
            let mut touching_loops = 0;
            for &start in &model_edges {
                if visited[start as usize] {
                    continue;
                }
                let mut current_loop = Vec::new();
                let mut touching = false;
                let mut edge = start;
                let mut vertex = board.edge_vertices[start as usize][1];
                loop {
                    visited[edge as usize] = true;
                    current_loop.push(edge);
                    touching |= touches_value(edge);
                    let mut next = u32::MAX;
                    for &e in board.vertex_edges(vertex as usize) {
                        if in_model[e as usize] && !visited[e as usize] {
                            next = e;
                            break;
                        }
                    }
                    if next == u32::MAX {
                        break;
                    }
                    let [a, b] = board.edge_vertices[next as usize];
                    vertex = if a == vertex { b } else { a };
                    edge = next;
                }
                touching_loops += touching as usize;
                loops.push(current_loop);
            }
            if touching_loops == 0 {
                return Some(true);
            }
            if touching_loops == 1 {
                // Another solution
                return Some(false);
            }

            // Exclude each loop, guarded by a value it breaks on its own
            for current_loop in &loops {
                let mut counts: Vec<(u32, u8)> = Vec::new();
                for &e in current_loop {
                    for &f in &board.edge_faces[e as usize] {
                        if f != u32::MAX && active[f as usize] {
                            match counts.iter_mut().find(|(face, _)| *face == f) {
                                Some((_, count)) => *count += 1,
                                None => counts.push((f, 1)),
                            }
                        }
                    }
                }
                let breaks = |f: usize| {
                    let count = counts
                        .iter()
                        .find(|(face, _)| *face as usize == f)
                        .map_or(0, |&(_, c)| c);
                    count != self.values[f]
                };
                let candidates = (0..board.num_faces).filter(|&f| active[f] && self.selectors[f] != NO_SELECTOR);
                let guard = candidates
                    .clone()
                    .filter(|&f| breaks(f))
                    .find(|&f| kept[f])
                    .or_else(|| candidates.clone().find(|&f| breaks(f)));
                let Some(guard) = guard else {
                    // (can't happen: with another touching loop, this one alone breaks some value)
                    return None;
                };
                clause.clear();
                clause.push(lit(self.selectors[guard], true));
                clause.extend(current_loop.iter().map(|&e| neg(e)));
                self.sat.add_clause(&clause);
            }
        }
        None
    }
}

/// Counts solutions (up to a maximum) for a series of related puzzles (puzzle generation), with one SAT solver
/// throughout, so what it learns carries over:
///
/// - Each (face, value) constraint is guarded by a selector variable (created when first needed), assumed true while
///   the face has that value
/// - Loops that aren't solutions are excluded with clauses guarded by the selector of a value the loop alone breaks, so
///   they stay excluded whenever that value is there
/// - Solutions found are only excluded for the current count (with a variable assumed for that count, then retired)
pub struct SolutionCounter {
    sat: Sat,
    num_edges: usize,
    // (face, value) => selector variable
    selectors: Vec<[u32; 16]>,
    /// Set when a count stopped at its round limit: the last model (possibly several loops), which differs from the known
    /// solution
    pub witness_model: Option<Vec<u32>>,
}

// Start over with a new solver past this many clauses (retired ones still take memory)
const MAX_COUNTER_CLAUSES: usize = 2_000_000;

impl SolutionCounter {
    pub fn new(board: &Board) -> SolutionCounter {
        let mut sat = Sat::new(board.num_edges);
        let mut clause: Vec<u32> = (0..board.num_edges as u32).map(pos).collect();
        sat.add_clause(&clause);
        for v in 0..board.num_vertices {
            let edges = board.vertex_edges(v);
            for_each_combination(edges, 3, &mut |combination| {
                let negated: Vec<u32> = combination.iter().map(|&e| neg(e)).collect();
                sat.add_clause(&negated);
            });
            for &e in edges {
                clause.clear();
                clause.push(neg(e));
                clause.extend(edges.iter().filter(|&&other| other != e).map(|&other| pos(other)));
                sat.add_clause(&clause);
            }
        }
        SolutionCounter {
            sat,
            num_edges: board.num_edges,
            selectors: vec![[NO_SELECTOR; 16]; board.num_faces],
            witness_model: None,
        }
    }

    fn selector(&mut self, board: &Board, f: usize, value: u8) -> u32 {
        let existing = self.selectors[f][value as usize];
        if existing != NO_SELECTOR {
            return existing;
        }
        let selector = self.sat.new_var();
        self.selectors[f][value as usize] = selector;
        let guard = lit(selector, true);
        let value = value as usize;
        let edges = board.face_edges(f);
        let sat = &mut self.sat;
        if value > edges.len() {
            sat.add_clause(&[guard]);
            return selector;
        }
        if value > 0 {
            for_each_combination(edges, edges.len() - value + 1, &mut |combination| {
                let mut guarded = vec![guard];
                guarded.extend(combination.iter().map(|&e| pos(e)));
                sat.add_clause(&guarded);
            });
        }
        for_each_combination(edges, value + 1, &mut |combination| {
            let mut guarded = vec![guard];
            guarded.extend(combination.iter().map(|&e| neg(e)));
            sat.add_clause(&guarded);
        });
        selector
    }

    /// Up to max_solutions solutions (each a loop's edges) for these values (0xff for none), or None if it gave up
    /// Up to max_solutions solutions (each a loop's edges) for these values (0xff for none), or None if it gave up. With a
    /// known solution, only finds others (see solve_excluding).
    pub fn solutions(
        &mut self,
        board: &Board,
        values: &[u8],
        max_solutions: usize,
        max_iterations: u64,
        known: Option<&[u32]>,
        // With a known solution: stop after this many models without finding another solution, keeping the last model
        // in witness_model
        witness_rounds: u64,
    ) -> Option<Vec<Vec<u32>>> {
        self.witness_model = None;
        if !values.iter().any(|&v| v != 0xff && v > 0) {
            return Some(Vec::new());
        }
        if self.sat.clause_count() > MAX_COUNTER_CLAUSES {
            *self = SolutionCounter::new(board);
        }
        let mut assumptions: Vec<u32> = Vec::new();
        for (f, &value) in values.iter().enumerate() {
            if value != 0xff {
                let selector = self.selector(board, f, value);
                assumptions.push(lit(selector, false));
            }
        }
        let call = self.sat.new_var();
        assumptions.push(lit(call, false));
        if let Some(known) = known {
            // For this count: not the known solution (nor it plus another loop), trying its opposite first
            let mut in_known = vec![false; self.num_edges];
            for &e in known {
                in_known[e as usize] = true;
            }
            let mut clause = vec![lit(call, true)];
            clause.extend((0..self.num_edges as u32).filter(|&e| !in_known[e as usize]).map(pos));
            self.sat.add_clause(&clause);
            let mut clause = vec![lit(call, true)];
            clause.extend(known.iter().map(|&e| neg(e)));
            self.sat.add_clause(&clause);
            for e in 0..self.num_edges as u32 {
                self.sat.set_preferred_value(e, in_known[e as usize]);
            }
        }

        let touches_value = |e: u32| {
            board.edge_faces[e as usize]
                .iter()
                .any(|&f| f != u32::MAX && values[f as usize] != 0xff)
        };
        let mut in_model = vec![false; self.num_edges];
        let mut visited = vec![false; self.num_edges];
        let mut solutions: Vec<Vec<u32>> = Vec::new();
        let mut clause: Vec<u32> = Vec::new();
        let mut result = None;
        let mut rounds_done = 0u64;
        #[cfg(feature = "profile")]
        let (start, start_conflicts) = (std::time::Instant::now(), self.sat.conflicts);
        #[cfg(feature = "profile")]
        let mut rounds = 0;
        for _ in 0..max_iterations {
            #[cfg(feature = "profile")]
            {
                rounds += 1;
            }
            match self.sat.solve_assuming(&assumptions, u64::MAX) {
                SolveStatus::Unsat => {
                    result = Some(solutions);
                    break;
                }
                SolveStatus::Unknown => break,
                SolveStatus::Sat => {}
            }
            let mut model_edges = Vec::new();
            for e in 0..self.num_edges as u32 {
                let black = self.sat.model_value(e);
                in_model[e as usize] = black;
                visited[e as usize] = false;
                if black {
                    model_edges.push(e);
                }
            }
            self.sat.reset_to_root();

            let mut loops: Vec<(Vec<u32>, bool)> = Vec::new();
            for &start in &model_edges {
                if visited[start as usize] {
                    continue;
                }
                let mut current_loop = Vec::new();
                let mut touching = false;
                let mut edge = start;
                let mut vertex = board.edge_vertices[start as usize][1];
                loop {
                    visited[edge as usize] = true;
                    current_loop.push(edge);
                    touching |= touches_value(edge);
                    let mut next = u32::MAX;
                    for &e in board.vertex_edges(vertex as usize) {
                        if in_model[e as usize] && !visited[e as usize] {
                            next = e;
                            break;
                        }
                    }
                    if next == u32::MAX {
                        break;
                    }
                    let [a, b] = board.edge_vertices[next as usize];
                    vertex = if a == vertex { b } else { a };
                    edge = next;
                }
                loops.push((current_loop, touching));
            }
            let touching_loops = loops.iter().filter(|(_, touching)| *touching).count();
            if touching_loops == 0 {
                // Then every value is 0
                result = Some(solutions);
                break;
            }
            for (current_loop, touching) in &loops {
                clause.clear();
                if touching_loops == 1 && *touching {
                    // A solution: excluded for this count only
                    solutions.push(current_loop.clone());
                    clause.push(lit(call, true));
                } else {
                    // Not a solution on its own: guarded by a value it breaks
                    let mut counts: Vec<(u32, u8)> = Vec::new();
                    for &e in current_loop {
                        for &f in &board.edge_faces[e as usize] {
                            if f != u32::MAX && values[f as usize] != 0xff {
                                match counts.iter_mut().find(|(face, _)| *face == f) {
                                    Some((_, count)) => *count += 1,
                                    None => counts.push((f, 1)),
                                }
                            }
                        }
                    }
                    let broken = (0..board.num_faces).find(|&f| {
                        values[f] != 0xff
                            && counts
                                .iter()
                                .find(|(face, _)| *face as usize == f)
                                .map_or(0, |&(_, c)| c)
                                != values[f]
                    });
                    match broken {
                        Some(f) => clause.push(lit(self.selectors[f][values[f] as usize], true)),
                        None => clause.push(lit(call, true)),
                    }
                }
                clause.extend(current_loop.iter().map(|&e| neg(e)));
                self.sat.add_clause(&clause);
            }
            if solutions.len() >= max_solutions {
                result = Some(solutions);
                break;
            }
            if known.is_some() && rounds_done + 1 >= witness_rounds {
                self.witness_model = Some(model_edges);
                result = Some(solutions);
                break;
            }
            rounds_done += 1;
        }
        #[cfg(feature = "profile")]
        eprintln!(
            "count: {} values, {} rounds, {} conflicts, {:.0} ms, {:?} solutions",
            values.iter().filter(|&&v| v != 0xff).count(),
            rounds,
            self.sat.conflicts - start_conflicts,
            start.elapsed().as_secs_f64() * 1000.0,
            result.as_ref().map(|r: &Vec<Vec<u32>>| r.len())
        );
        // Retire this count's variable (its clauses no longer apply)
        self.sat.add_clause(&[lit(call, true)]);
        result
    }
}
