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
