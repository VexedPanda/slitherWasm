//! Slitherlink loop solver: constraint propagation plus backtracking, specialized for the puzzle.
//!
//! A solution is a single closed loop of edges such that every face with a value has exactly that many loop edges, the
//! loop contains every given black edge and no given red edge, and the loop touches at least one face with a value. This
//! matches what the JS `satSolve` counts (which only counts a SAT solution when exactly one of its loops touches a value,
//! and excludes every loop it sees).
//!
//! Partial black edges form vertex-disjoint paths ("chains"). `partner[v]` links the two ends of each chain, so adding an
//! edge that would close a chain into a loop is detected in O(1), and only allowed when that loop completes the puzzle.

use std::collections::HashSet;

pub const WHITE: u8 = 0;
pub const BLACK: u8 = 1;
pub const RED: u8 = 2;

pub const NO_VALUE: i8 = -1;

// Probe (see Solver::probe) at search nodes up to this depth, as well as at the start
const PROBE_DEPTH: u32 = 3;

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
    // The regions on either side of each edge, for inside/outside parity: faces, then one region per boundary (the
    // outside, and each hole, which can each be inside or outside the loop independently)
    pub edge_regions: Vec<[u32; 2]>,
    pub num_regions: usize,
}

impl Board {
    pub fn new(
        num_vertices: usize,
        edge_vertices: &[u32],
        face_edge_offsets: &[u32],
        face_edges: &[u32],
    ) -> Board {
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

        // Group boundary edges into boundaries (connected through shared vertices)
        let mut boundary_parent: Vec<u32> = (0..num_vertices as u32).collect();
        fn root(parent: &mut [u32], mut x: u32) -> u32 {
            while parent[x as usize] != x {
                parent[x as usize] = parent[parent[x as usize] as usize];
                x = parent[x as usize];
            }
            x
        }
        for e in 0..num_edges {
            if edge_faces[e][1] == u32::MAX {
                let ra = root(&mut boundary_parent, ev[e][0]);
                let rb = root(&mut boundary_parent, ev[e][1]);
                boundary_parent[ra as usize] = rb;
            }
        }
        let mut boundary_region = vec![u32::MAX; num_vertices];
        let mut num_regions = num_faces;
        let mut edge_regions = vec![[0u32; 2]; num_edges];
        for e in 0..num_edges {
            let [f0, f1] = edge_faces[e];
            let other = if f1 != u32::MAX {
                f1
            } else {
                let r = root(&mut boundary_parent, ev[e][0]) as usize;
                if boundary_region[r] == u32::MAX {
                    boundary_region[r] = num_regions as u32;
                    num_regions += 1;
                }
                boundary_region[r]
            };
            edge_regions[e] = [f0, other];
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
            edge_regions,
            num_regions,
        }
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
    // Gave up after the node budget
    BudgetExceeded,
}

// One assignment on the trail, with what is needed to undo it
struct TrailEntry {
    edge: u32,
    // For black edges: chain ends whose partner was overwritten, and their old partners
    pa: u32,
    pa_old: u32,
    pb: u32,
    pb_old: u32,
    chains_old: u32,
    solved_old: bool,
    // Parity union: the root that was attached (u32::MAX if none), and whether its new root's rank was bumped
    uf_child: u32,
    uf_rank_bumped: bool,
}

pub struct Solver<'a> {
    board: &'a Board,
    face_values: Vec<i8>,
    state: Vec<u8>,
    vblack: Vec<u8>,
    vwhite: Vec<u8>,
    fblack: Vec<u16>,
    fwhite: Vec<u16>,
    partner: Vec<u32>,
    chains: u32,
    // Faces with a value whose black count isn't (yet) equal to it
    unsatisfied_faces: u32,
    has_positive_value: bool,
    solved: bool,
    trail: Vec<TrailEntry>,
    queue_vertices: Vec<u32>,
    queue_faces: Vec<u32>,
    in_vertex_queue: Vec<bool>,
    in_face_queue: Vec<bool>,
    nodes: u64,
    node_budget: u64,
    max_solutions: usize,
    solutions: Vec<Vec<u32>>,
    // Scratch for the connectivity check
    component: Vec<u32>,
    stack: Vec<u32>,
    // Inside/outside parity between regions (union-find without path compression, so it can be undone): a black edge
    // separates regions of different parity, a red edge joins regions of the same parity
    uf_parent: Vec<u32>,
    uf_parity: Vec<u8>,
    uf_rank: Vec<u8>,
    // Restarts: each run of the search is randomized and limited, so one bad early choice can't trap it (see solve)
    run_nodes: u64,
    run_node_limit: u64,
    rng: u64,
    solution_keys: HashSet<Vec<u32>>,
}

enum Abort {
    Budget,
    Restart,
}

// Node limit of the first search run (each restart doubles it)
const FIRST_RUN_NODE_LIMIT: u64 = 256;

impl<'a> Solver<'a> {
    pub fn solve(
        board: &'a Board,
        face_values: &[i8],
        edge_states: &[u8],
        max_solutions: usize,
        node_budget: u64,
    ) -> (SolveResult, u64) {
        let mut solver = Solver {
            board,
            face_values: face_values.to_vec(),
            state: vec![WHITE; board.num_edges],
            vblack: vec![0; board.num_vertices],
            vwhite: vec![0; board.num_vertices],
            fblack: vec![0; board.num_faces],
            fwhite: vec![0; board.num_faces],
            partner: (0..board.num_vertices as u32).collect(),
            chains: 0,
            unsatisfied_faces: 0,
            has_positive_value: face_values.iter().any(|&v| v > 0),
            solved: false,
            trail: Vec::with_capacity(board.num_edges),
            queue_vertices: Vec::new(),
            queue_faces: Vec::new(),
            in_vertex_queue: vec![false; board.num_vertices],
            in_face_queue: vec![false; board.num_faces],
            nodes: 0,
            node_budget,
            max_solutions: max_solutions.max(1),
            solutions: Vec::new(),
            component: vec![0; board.num_vertices],
            stack: Vec::new(),
            uf_parent: (0..board.num_regions as u32).collect(),
            uf_parity: vec![0; board.num_regions],
            uf_rank: vec![0; board.num_regions],
            run_nodes: 0,
            run_node_limit: u64::MAX,
            rng: 0x9E37_79B9_7F4A_7C15,
            solution_keys: HashSet::new(),
        };

        if !solver.has_positive_value {
            // Every loop would either break a 0 or touch no value
            return (SolveResult::Solutions(Vec::new()), 0);
        }

        for v in 0..board.num_vertices {
            solver.vwhite[v] = board.vertex_edges(v).len() as u8;
        }
        for f in 0..board.num_faces {
            solver.fwhite[f] = board.face_edges(f).len() as u16;
            if face_values[f] != NO_VALUE && face_values[f] != 0 {
                solver.unsatisfied_faces += 1;
            }
        }

        // Apply the given edges, then propagate everything once
        let mut ok = true;
        for e in 0..board.num_edges {
            match edge_states[e] {
                BLACK => ok = ok && solver.assign(e as u32, BLACK),
                RED => ok = ok && solver.assign(e as u32, RED),
                _ => {}
            }
        }
        if ok {
            for v in 0..board.num_vertices {
                solver.enqueue_vertex(v as u32);
            }
            for f in 0..board.num_faces {
                solver.enqueue_face(f as u32);
            }
            ok = solver.propagate();
        }

        if ok {
            ok = solver.probe();
        }
        if ok {
            // Randomized runs with doubling node limits. Solutions are kept (deduplicated) across runs, and a run that
            // finishes without hitting its limit has seen every solution, so the result is exact.
            let root = solver.trail.len();
            let mut limit = FIRST_RUN_NODE_LIMIT;
            loop {
                solver.run_nodes = 0;
                solver.run_node_limit = limit;
                let result = solver.search(0);
                solver.undo_to(root);
                solver.clear_queues();
                match result {
                    Ok(()) => break,
                    Err(Abort::Budget) => return (SolveResult::BudgetExceeded, solver.nodes),
                    Err(Abort::Restart) => {
                        if solver.solutions.len() >= solver.max_solutions {
                            break;
                        }
                        limit = limit.saturating_mul(2);
                    }
                }
            }
        }
        (SolveResult::Solutions(solver.solutions), solver.nodes)
    }

    #[inline]
    fn other_vertex(&self, e: u32, v: u32) -> u32 {
        let [a, b] = self.board.edge_vertices[e as usize];
        if a == v { b } else { a }
    }

    #[inline]
    fn enqueue_vertex(&mut self, v: u32) {
        if !self.in_vertex_queue[v as usize] {
            self.in_vertex_queue[v as usize] = true;
            self.queue_vertices.push(v);
        }
    }

    #[inline]
    fn enqueue_face(&mut self, f: u32) {
        if f != u32::MAX && !self.in_face_queue[f as usize] {
            self.in_face_queue[f as usize] = true;
            self.queue_faces.push(f);
        }
    }

    fn clear_queues(&mut self) {
        for v in self.queue_vertices.drain(..) {
            self.in_vertex_queue[v as usize] = false;
        }
        for f in self.queue_faces.drain(..) {
            self.in_face_queue[f as usize] = false;
        }
    }

    // Whether adding black edge e (between chain ends) closes a loop that completes the puzzle
    fn closing_completes(&self, e: u32) -> bool {
        if self.chains != 1 {
            return false;
        }
        // Every face with a value must be exactly satisfied once e is black
        let mut unsatisfied = self.unsatisfied_faces as i32;
        for &f in &self.board.edge_faces[e as usize] {
            if f == u32::MAX {
                continue;
            }
            let value = self.face_values[f as usize];
            if value == NO_VALUE {
                continue;
            }
            let before = self.fblack[f as usize] as i32;
            let was = before == value as i32;
            let now = before + 1 == value as i32;
            unsatisfied += (was as i32) - (now as i32);
        }
        unsatisfied == 0
    }

    // Assigns a white edge. Returns false on an immediate contradiction (the assignment is still on the trail).
    fn assign(&mut self, e: u32, value: u8) -> bool {
        let ei = e as usize;
        debug_assert!(self.state[ei] == WHITE);
        let [a, b] = self.board.edge_vertices[ei];
        let mut entry = TrailEntry {
            edge: e,
            pa: u32::MAX,
            pa_old: 0,
            pb: u32::MAX,
            pb_old: 0,
            chains_old: self.chains,
            solved_old: self.solved,
            uf_child: u32::MAX,
            uf_rank_bumped: false,
        };
        let mut ok = true;

        self.state[ei] = value;
        self.vwhite[a as usize] -= 1;
        self.vwhite[b as usize] -= 1;
        for &f in &self.board.edge_faces[ei] {
            if f != u32::MAX {
                self.fwhite[f as usize] -= 1;
            }
        }

        if value == BLACK {
            let closes = self.vblack[a as usize] == 1 && self.vblack[b as usize] == 1 && self.partner[a as usize] == b;
            if closes {
                if self.closing_completes(e) {
                    self.solved = true;
                    self.chains = 0;
                } else {
                    ok = false;
                }
            } else if self.vblack[a as usize] >= 2 || self.vblack[b as usize] >= 2 {
                ok = false;
            } else {
                // Join the chains (an isolated vertex is its own partner)
                let pa = self.partner[a as usize];
                let pb = self.partner[b as usize];
                entry.pa = pa;
                entry.pa_old = self.partner[pa as usize];
                entry.pb = pb;
                entry.pb_old = self.partner[pb as usize];
                self.partner[pa as usize] = pb;
                self.partner[pb as usize] = pa;
                let ends = (self.vblack[a as usize] == 1) as u32 + (self.vblack[b as usize] == 1) as u32;
                // 0 existing ends: new chain; 1: extends a chain; 2: merges two chains
                self.chains = self.chains + 1 - ends;
            }

            self.vblack[a as usize] += 1;
            self.vblack[b as usize] += 1;
            for &f in &self.board.edge_faces[ei] {
                if f != u32::MAX {
                    let value = self.face_values[f as usize];
                    if value != NO_VALUE {
                        let before = self.fblack[f as usize] as i32;
                        if before == value as i32 {
                            self.unsatisfied_faces += 1;
                        } else if before + 1 == value as i32 {
                            self.unsatisfied_faces -= 1;
                        }
                    }
                    self.fblack[f as usize] += 1;
                }
            }
        }

        // Black edges separate regions of different parity, red edges regions of the same parity
        let [r0, r1] = self.board.edge_regions[ei];
        let (root0, parity0) = self.find(r0);
        let (root1, parity1) = self.find(r1);
        let difference = (value == BLACK) as u8;
        if root0 == root1 {
            if parity0 ^ parity1 != difference {
                ok = false;
            }
        } else {
            let (child, parent) = if self.uf_rank[root0 as usize] < self.uf_rank[root1 as usize] {
                (root0, root1)
            } else {
                (root1, root0)
            };
            self.uf_parent[child as usize] = parent;
            self.uf_parity[child as usize] = parity0 ^ parity1 ^ difference;
            entry.uf_child = child;
            if self.uf_rank[child as usize] == self.uf_rank[parent as usize] {
                self.uf_rank[parent as usize] += 1;
                entry.uf_rank_bumped = true;
            }
        }

        self.trail.push(entry);

        self.enqueue_vertex(a);
        self.enqueue_vertex(b);
        let [f0, f1] = self.board.edge_faces[ei];
        self.enqueue_face(f0);
        self.enqueue_face(f1);
        ok
    }

    fn undo_to(&mut self, mark: usize) {
        while self.trail.len() > mark {
            let entry = self.trail.pop().unwrap();
            let ei = entry.edge as usize;
            let value = self.state[ei];
            let [a, b] = self.board.edge_vertices[ei];
            if value == BLACK {
                self.vblack[a as usize] -= 1;
                self.vblack[b as usize] -= 1;
                for &f in &self.board.edge_faces[ei] {
                    if f != u32::MAX {
                        self.fblack[f as usize] -= 1;
                        let value = self.face_values[f as usize];
                        if value != NO_VALUE {
                            let after = self.fblack[f as usize] as i32;
                            if after == value as i32 {
                                self.unsatisfied_faces -= 1;
                            } else if after + 1 == value as i32 {
                                self.unsatisfied_faces += 1;
                            }
                        }
                    }
                }
                if entry.pb != u32::MAX {
                    self.partner[entry.pb as usize] = entry.pb_old;
                    self.partner[entry.pa as usize] = entry.pa_old;
                }
            }
            if entry.uf_child != u32::MAX {
                let child = entry.uf_child as usize;
                if entry.uf_rank_bumped {
                    let parent = self.uf_parent[child] as usize;
                    self.uf_rank[parent] -= 1;
                }
                self.uf_parent[child] = child as u32;
                self.uf_parity[child] = 0;
            }
            self.chains = entry.chains_old;
            self.solved = entry.solved_old;
            self.state[ei] = WHITE;
            self.vwhite[a as usize] += 1;
            self.vwhite[b as usize] += 1;
            for &f in &self.board.edge_faces[ei] {
                if f != u32::MAX {
                    self.fwhite[f as usize] += 1;
                }
            }
        }
    }

    #[inline]
    fn find(&self, mut x: u32) -> (u32, u8) {
        let mut parity = 0;
        while self.uf_parent[x as usize] != x {
            parity ^= self.uf_parity[x as usize];
            x = self.uf_parent[x as usize];
        }
        (x, parity)
    }

    // The state a white edge must have when its two regions' relative parity is already known
    #[inline]
    fn parity_forced(&self, e: u32) -> Option<u8> {
        let [r0, r1] = self.board.edge_regions[e as usize];
        let (root0, parity0) = self.find(r0);
        let (root1, parity1) = self.find(r1);
        if root0 == root1 {
            Some(if parity0 ^ parity1 == 1 { BLACK } else { RED })
        } else {
            None
        }
    }

    // Assigns and propagates, leaving the queues clear. Returns false on a contradiction (the trail still needs undoing).
    fn try_assign(&mut self, e: u32, value: u8) -> bool {
        let ok = self.assign(e, value) && self.propagate();
        if !ok {
            self.clear_queues();
        }
        ok
    }

    /// Failed-literal probing: an edge whose black (or red) assignment leads to a contradiction by propagation must be
    /// the other state. Repeats until nothing changes. Returns false if the state has no solutions.
    fn probe(&mut self) -> bool {
        loop {
            if self.solved {
                return true;
            }
            let mut changed = false;
            for e in 0..self.board.num_edges as u32 {
                if self.state[e as usize] != WHITE {
                    continue;
                }
                let mark = self.trail.len();
                let black_ok = self.try_assign(e, BLACK);
                self.undo_to(mark);
                let red_ok = self.try_assign(e, RED);
                self.undo_to(mark);
                match (black_ok, red_ok) {
                    (false, false) => return false,
                    (false, true) => {
                        if !self.try_assign(e, RED) {
                            return false;
                        }
                        changed = true;
                    }
                    (true, false) => {
                        if !self.try_assign(e, BLACK) {
                            return false;
                        }
                        changed = true;
                    }
                    _ => {}
                }
                if self.solved {
                    return true;
                }
            }
            if !changed {
                return true;
            }
        }
    }

    // Applies forced assignments until nothing changes. Returns false on a contradiction.
    fn propagate(&mut self) -> bool {
        loop {
            if self.solved {
                // The loop is complete: everything else is red
                self.clear_queues();
                return true;
            }
            if let Some(v) = self.queue_vertices.pop() {
                self.in_vertex_queue[v as usize] = false;
                if !self.propagate_vertex(v) {
                    self.clear_queues();
                    return false;
                }
            } else if let Some(f) = self.queue_faces.pop() {
                self.in_face_queue[f as usize] = false;
                if !self.propagate_face(f) {
                    self.clear_queues();
                    return false;
                }
            } else {
                return true;
            }
        }
    }

    fn propagate_vertex(&mut self, v: u32) -> bool {
        let vi = v as usize;
        for i in self.board.vertex_edge_offsets[vi] as usize..self.board.vertex_edge_offsets[vi + 1] as usize {
            let e = self.board.vertex_edges[i];
            if self.state[e as usize] == WHITE {
                if let Some(value) = self.parity_forced(e) {
                    if !self.assign(e, value) {
                        return false;
                    }
                }
            }
        }
        let black = self.vblack[vi];
        let white = self.vwhite[vi];
        if black > 2 {
            return false;
        }
        if white == 0 {
            return black != 1;
        }
        let range = self.board.vertex_edge_offsets[vi] as usize..self.board.vertex_edge_offsets[vi + 1] as usize;
        match black {
            2 => {
                for i in range {
                    let e = self.board.vertex_edges[i];
                    if self.state[e as usize] == WHITE && !self.assign(e, RED) {
                        return false;
                    }
                }
            }
            1 => {
                if white == 1 {
                    for i in range {
                        let e = self.board.vertex_edges[i];
                        if self.state[e as usize] == WHITE {
                            return self.assign(e, BLACK);
                        }
                    }
                } else {
                    // An edge that would close the chain into a loop is only possible if that finishes the puzzle
                    let p = self.partner[vi];
                    for i in range {
                        let e = self.board.vertex_edges[i];
                        if self.state[e as usize] == WHITE
                            && self.other_vertex(e, v) == p
                            && !self.closing_completes(e)
                            && !self.assign(e, RED)
                        {
                            return false;
                        }
                    }
                }
            }
            _ => {
                if white == 1 {
                    for i in range {
                        let e = self.board.vertex_edges[i];
                        if self.state[e as usize] == WHITE {
                            return self.assign(e, RED);
                        }
                    }
                }
            }
        }
        true
    }

    fn propagate_face(&mut self, f: u32) -> bool {
        let fi = f as usize;
        for i in self.board.face_edge_offsets[fi] as usize..self.board.face_edge_offsets[fi + 1] as usize {
            let e = self.board.face_edges[i];
            if self.state[e as usize] == WHITE {
                if let Some(value) = self.parity_forced(e) {
                    if !self.assign(e, value) {
                        return false;
                    }
                }
            }
        }
        let value = self.face_values[fi];
        if value == NO_VALUE {
            return true;
        }
        let value = value as u16;
        let black = self.fblack[fi];
        let white = self.fwhite[fi];
        if black > value || black + white < value {
            return false;
        }
        if white == 0 {
            return true;
        }
        let fill = if black == value {
            RED
        } else if black + white == value {
            BLACK
        } else {
            return true;
        };
        let range = self.board.face_edge_offsets[fi] as usize..self.board.face_edge_offsets[fi + 1] as usize;
        for i in range {
            let e = self.board.face_edges[i];
            if self.state[e as usize] == WHITE && !self.assign(e, fill) {
                return false;
            }
        }
        true
    }

    // All black edges and all unsatisfied faces must be reachable from each other through non-red edges
    fn connected(&mut self) -> bool {
        let n = self.board.num_vertices;
        let mut start = u32::MAX;
        for v in 0..n {
            if self.vblack[v] > 0 {
                start = v as u32;
                break;
            }
        }
        if start == u32::MAX {
            return true;
        }
        let stamp = self.nodes as u32 + 1;
        self.stack.clear();
        self.stack.push(start);
        self.component[start as usize] = stamp;
        while let Some(v) = self.stack.pop() {
            let vi = v as usize;
            for i in self.board.vertex_edge_offsets[vi] as usize..self.board.vertex_edge_offsets[vi + 1] as usize {
                let e = self.board.vertex_edges[i];
                if self.state[e as usize] != RED {
                    let u = self.other_vertex(e, v);
                    if self.component[u as usize] != stamp {
                        self.component[u as usize] = stamp;
                        self.stack.push(u);
                    }
                }
            }
        }
        for v in 0..n {
            if self.vblack[v] > 0 && self.component[v] != stamp {
                return false;
            }
        }
        for f in 0..self.board.num_faces {
            let value = self.face_values[f];
            if value > 0 && (self.fblack[f] as i32) < value as i32 {
                // Some non-red edge of the face must be reachable
                let reachable = self.board.face_edges(f).iter().any(|&e| {
                    self.state[e as usize] != RED && {
                        let [a, _] = self.board.edge_vertices[e as usize];
                        self.component[a as usize] == stamp
                    }
                });
                if !reachable {
                    return false;
                }
            }
        }
        true
    }

    fn record_solution(&mut self) {
        // Walk the loop from any black edge
        let mut loop_edges = Vec::new();
        let mut key: Vec<u32> = (0..self.board.num_edges as u32)
            .filter(|&e| self.state[e as usize] == BLACK)
            .collect();
        key.sort_unstable();
        if !self.solution_keys.insert(key) {
            // Already found in an earlier run
            return;
        }
        let first = match self.state.iter().position(|&s| s == BLACK) {
            Some(e) => e as u32,
            None => return,
        };
        let mut edge = first;
        let mut vertex = self.board.edge_vertices[first as usize][1];
        loop {
            loop_edges.push(edge);
            let mut next = u32::MAX;
            for &e in self.board.vertex_edges(vertex as usize) {
                if e != edge && self.state[e as usize] == BLACK {
                    next = e;
                    break;
                }
            }
            if next == u32::MAX || next == first {
                break;
            }
            vertex = self.other_vertex(next, vertex);
            edge = next;
        }
        self.solutions.push(loop_edges);
    }

    // Picks a white edge to branch on, preferring chain ends with the fewest options
    fn choose_edge(&mut self) -> u32 {
        let mut best = u32::MAX;
        let mut best_score = u32::MAX;
        let n = self.board.num_vertices;
        let offset = (self.random() % n as u64) as usize;
        for i in 0..n {
            let v = (i + offset) % n;
            if self.vblack[v] == 1 {
                let white = self.vwhite[v] as u32;
                if white < best_score {
                    for &e in self.board.vertex_edges(v) {
                        if self.state[e as usize] == WHITE {
                            best = e;
                            best_score = white;
                            break;
                        }
                    }
                    if best_score <= 2 {
                        break;
                    }
                }
            }
        }
        if best != u32::MAX {
            return best;
        }
        // No chains yet: start on the most constrained unsatisfied face
        let mut best_face_score = i32::MAX;
        for f in 0..self.board.num_faces {
            let value = self.face_values[f];
            if value > 0 && (self.fblack[f] as i32) < value as i32 {
                let score = self.fwhite[f] as i32 - (value as i32 - self.fblack[f] as i32);
                if score < best_face_score {
                    for &e in self.board.face_edges(f) {
                        if self.state[e as usize] == WHITE {
                            best = e;
                            best_face_score = score;
                            break;
                        }
                    }
                }
            }
        }
        best
    }

    // xorshift64*
    fn random(&mut self) -> u64 {
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    // Err when the node budget (or this run's node limit) runs out
    fn search(&mut self, depth: u32) -> Result<(), Abort> {
        self.nodes += 1;
        self.run_nodes += 1;
        if self.nodes > self.node_budget {
            return Err(Abort::Budget);
        }
        if self.run_nodes > self.run_node_limit {
            return Err(Abort::Restart);
        }

        if self.solved {
            self.record_solution();
            return Ok(());
        }
        if !self.connected() {
            return Ok(());
        }
        if depth > 0 && depth <= PROBE_DEPTH {
            if !self.probe() {
                return Ok(());
            }
            if self.solved {
                self.record_solution();
                return Ok(());
            }
        }

        let e = self.choose_edge();
        if e == u32::MAX {
            return Ok(());
        }

        let order = if self.random() & 1 == 0 { [BLACK, RED] } else { [RED, BLACK] };
        for value in order {
            let mark = self.trail.len();
            if self.assign(e, value) && self.propagate() {
                self.search(depth + 1)?;
            } else {
                self.clear_queues();
            }
            self.undo_to(mark);
            if self.solutions.len() >= self.max_solutions {
                return Ok(());
            }
        }
        Ok(())
    }
}
