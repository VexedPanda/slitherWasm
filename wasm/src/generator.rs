//! Puzzle generation (generateAdditiveConstrained in TS), run in bounded steps so the caller can show progress, yield and
//! interrupt (see slitherCore.ts):
//!
//! 1. Additive: in random face order, give each face a random value (not "all edges") that still leaves a solution, until
//!    the solution is unique (giving up and starting over after a pass without one).
//! 2. Unless there's no difficulty limit: fill every face with its value in the solution, and start over if a face has all
//!    of its edges in the loop, or the difficulty check can't solve it.
//! 3. Minimize: in random face order, remove each value if the puzzle stays unique and passes the difficulty check.
//!
//! Progress events (as in TS): define (a face got a value), minimize (a face was processed: its value was removed, or
//! kept) and reset (starting over). An interrupted generation can be resumed from its events.

use crate::cansolve::{self, RuleSet};
use crate::engine;
use crate::solver::{Board, SolutionCounter, Uniqueness};
use crate::topology::Topology;

pub const NO_VALUE: u8 = 0xff;

pub const EVENT_DEFINE: u32 = 0;
pub const EVENT_MINIMIZE: u32 = 1;
pub const EVENT_RESET: u32 = 2;

/// Which difficulty check to use
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Check {
    None,
    // EASY / MEDIUM (cansolve.rs)
    Simple,
    // HARD (engine.rs)
    Hard,
}

// Most recent loops are checked first, and the oldest are dropped past this (as in KnownSolutions.ts)
const MAX_KNOWN_SOLUTIONS: usize = 200;
// Models tried while looking for another solution, before using the last one (not a solution) as the next witness
// Random loops: one in this many faces next to the growing region is kept outside, and regions smaller than 1/this of
// the board are retried
const LOOP_OUTSIDE_ODDS: u64 = 3;
const LOOP_MIN_INSIDE_DIVISOR: usize = 3;
// Random loops grow towards faces more than this many faces away from a face the loop touches (far from the loop, faces
// need few values, leaving large areas without values)
const LOOP_MAX_GAP: u32 = 1;
// Faces per minimization cluster (see Generator::start_minimize)
const MINIMIZE_CLUSTER_SIZE: usize = 32;
const WITNESS_ROUNDS: u64 = 256;
// The most values added at once from a witness solution
const MAX_WITNESS_VALUES: usize = 256;
// Witness values added at once: this fraction (1 / WITNESS_DIVISOR) of the faces where the witnesses differ
const WITNESS_DIVISOR: usize = 2;
// Values added at once while guided by the sound rules: this fraction (1 / GUIDED_DIVISOR) of the candidate faces
const GUIDED_DIVISOR: usize = 8;
// Models a minimization uniqueness check may exclude before giving up (keeping the value)
const UNIQUENESS_ROUNDS: u64 = 100;
// The most values minimization tries to remove at once
const MAX_MINIMIZE_BLOCK: usize = 32;
// Give up on an additive pass after this many (as in generateFaceAdditive.ts)
const MAX_ADDITIVE_ATTEMPTS: u32 = 100;

/// The difficulty's rules, and optionally an easier difficulty's (a subset of them, see can_solve)
#[derive(Clone, Copy)]
pub struct Rules<'a> {
    pub main: Option<&'a RuleSet>,
    pub easier: Option<&'a RuleSet>,
    /// Rules without uniqueness assumptions (no highlander rules), EASY/MEDIUM-style: if they solve a puzzle, it's
    /// unique, which avoids most SAT checks (see cansolve::deduce)
    pub sound: Option<&'a RuleSet>,
    /// Whether puzzles the sound rules solve also pass the difficulty check (the sound rules are a subset of its rules)
    pub sound_implies_difficulty: bool,
    /// Whether minimization only removes values while the sound rules still solve the puzzle (no SAT), so the puzzle
    /// is solvable without the uniqueness-assuming (highlander) rules. Otherwise, when they can't, it falls back to the
    /// difficulty check and SAT. With sound_implies_difficulty, the sound rules are then the whole difficulty check (the
    /// main and easier rules are unused).
    pub sound_only: bool,
}

pub enum Status {
    Running,
    Done,
    Failed,
}

enum Phase {
    // Additive pass: the face order and position in it
    Additive {
        order: Vec<u32>,
        position: usize,
        attempts: u32,
    },
    // Adding values from a known solution, where the sound rules can't yet tell (see guided_step)
    Guided {
        attempts: u32,
    },
    Minimize {
        order: Vec<u32>,
        position: usize,
    },
    Done,
    Failed,
}

pub struct Generator {
    check: Check,
    // With sound rules: start from a random board-filling loop and add its values until they solve it (no SAT counting)
    loop_first: bool,
    rng: u64,
    face_values: Vec<u8>,
    // The solution's edges (once the puzzle is unique)
    solution: Vec<u32>,
    phase: Phase,
    pub events: Vec<u32>,
    // Loops found by SAT solves: each loop's number of edges around each face, and its sorted edges
    known: Vec<(Vec<u8>, Vec<u32>)>,
    // Minimization: the values it started from, faces whose values are kept, and the incremental uniqueness checker
    minimize_start_values: Vec<u8>,
    kept: Vec<bool>,
    uniqueness: Option<Uniqueness>,
    // Solution counting while adding values
    counter: Option<SolutionCounter>,
    // Values to try removing at once while minimizing
    block: usize,
    // Loop-first minimization goes through clusters of faces (ending at these positions), checking removals from
    // a checkpoint of the deduction without the current cluster's values (see cansolve::Checkpoint)
    cluster_ends: Vec<usize>,
    checkpoint: Option<(usize, cansolve::Checkpoint)>,
    // Two solutions of the current values, while adding values (the second may instead be a model with several loops,
    // in which case only the first is a solution)
    witnesses: Option<(Vec<u32>, Vec<u32>)>,
    second_witness_is_solution: bool,
}

impl Generator {
    /// Starts generating, optionally resuming from earlier progress events (type, face, value triples)
    pub fn new(
        board: &Board,
        topology: Option<&Topology>,
        rules: Rules,
        check: Check,
        seed: u64,
        resume: &[u32],
    ) -> Generator {
        let mut generator = Generator {
            check,
            loop_first: rules.sound.is_some(),
            rng: seed | 1,
            face_values: vec![NO_VALUE; board.num_faces],
            solution: Vec::new(),
            phase: Phase::Failed,
            events: Vec::new(),
            known: Vec::new(),
            minimize_start_values: Vec::new(),
            kept: vec![false; board.num_faces],
            uniqueness: None,
            counter: None,
            block: 1,
            cluster_ends: Vec::new(),
            checkpoint: None,
            witnesses: None,
            second_witness_is_solution: true,
        };
        generator.resume(board, topology, rules, resume);
        generator
    }

    fn random(&mut self) -> u64 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn shuffle(&mut self, items: &mut [u32]) {
        for i in (1..items.len()).rev() {
            let j = (self.random() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }

    fn emit(&mut self, kind: u32, face: u32, value: u8) {
        self.events.extend_from_slice(&[kind, face, value as u32]);
    }

    // Continues from earlier progress: minimizing (if it got that far, and the puzzle still checks out), otherwise
    // adding values to the faces defined so far
    fn resume(&mut self, board: &Board, topology: Option<&Topology>, rules: Rules, events: &[u32]) {
        let mut defined = vec![NO_VALUE; board.num_faces];
        let mut minimized = vec![None; board.num_faces];
        for event in events.chunks(3) {
            let (kind, face, value) = (event[0], event[1] as usize, event[2] as u8);
            match kind {
                EVENT_RESET => {
                    defined.fill(NO_VALUE);
                    minimized.fill(None);
                }
                EVENT_DEFINE if face < board.num_faces => defined[face] = value,
                EVENT_MINIMIZE if face < board.num_faces => minimized[face] = Some(value),
                _ => {}
            }
        }

        if minimized.iter().any(|m| m.is_some()) {
            let values: Vec<u8> = (0..board.num_faces)
                .map(|f| minimized[f].unwrap_or(defined[f]))
                .collect();
            let solutions = self.solutions(board, &values);
            if let Some(solutions) = solutions
                && solutions.len() == 1
                && self.can_solve(board, topology, rules, &values)
            {
                self.face_values = values;
                self.solution = solutions[0].clone();
                let mut order: Vec<u32> = (0..board.num_faces as u32)
                    .filter(|&f| minimized[f as usize].is_none())
                    .collect();
                self.shuffle(&mut order);
                self.start_minimize(order);
                return;
            }
            // Couldn't resume
            self.emit(EVENT_RESET, 0, 0);
            defined.fill(NO_VALUE);
        }

        if self.loop_first && defined.iter().any(|&v| v != NO_VALUE) {
            // The loop isn't in the events, and adding values is quick: start over
            self.emit(EVENT_RESET, 0, 0);
        } else if defined.iter().any(|&v| v != NO_VALUE) {
            match self.solutions(board, &defined) {
                Some(solutions) if !solutions.is_empty() => {
                    self.face_values = defined;
                    if solutions.len() == 1 {
                        self.solution = solutions[0].clone();
                        self.finish_additive(board, topology, rules);
                    } else {
                        let mut order: Vec<u32> = (0..board.num_faces as u32)
                            .filter(|&f| self.face_values[f as usize] == NO_VALUE)
                            .collect();
                        self.shuffle(&mut order);
                        self.phase = Phase::Additive {
                            order,
                            position: 0,
                            attempts: 1,
                        };
                    }
                    return;
                }
                _ => self.emit(EVENT_RESET, 0, 0),
            }
        }
        self.start_additive(board, 1);
    }

    fn start_additive(&mut self, board: &Board, attempts: u32) {
        if attempts > MAX_ADDITIVE_ATTEMPTS {
            self.phase = Phase::Failed;
            return;
        }
        self.face_values.fill(NO_VALUE);
        self.witnesses = None;
        if self.loop_first {
            self.solution = self.random_loop(board);
            self.phase = Phase::Guided { attempts };
            return;
        }
        let mut order: Vec<u32> = (0..board.num_faces as u32).collect();
        self.shuffle(&mut order);
        self.phase = Phase::Additive {
            order,
            position: 0,
            attempts,
        };
    }

    // A random loop filling the board (as in Loopy): grows a region of faces from a random face, adding random
    // neighboring faces while its boundary stays a single loop, until none can be added. Some neighboring faces are
    // instead kept outside (so the outside also reaches across the board, and the loop winds through it). When none can
    // be added while faces are far from the loop (walled off by faces kept outside), the neighboring faces near them are
    // tried again, growing the region towards them. Retries while the region is small. Returns the loop's sorted edges.
    fn random_loop(&mut self, board: &Board) -> Vec<u32> {
        loop {
            let (edges, inside) = self.random_region_loop(board);
            if inside * LOOP_MIN_INSIDE_DIVISOR >= board.num_faces {
                return edges;
            }
        }
    }

    // For each face, how many faces away it is from the nearest of these faces (u32::MAX if none is reachable)
    fn face_distances(board: &Board, sources: impl Iterator<Item = u32>) -> Vec<u32> {
        let mut distance = vec![u32::MAX; board.num_faces];
        let mut queue = std::collections::VecDeque::new();
        for f in sources {
            if distance[f as usize] == u32::MAX {
                distance[f as usize] = 0;
                queue.push_back(f);
            }
        }
        while let Some(f) = queue.pop_front() {
            for &e in board.face_edges_of(f as usize) {
                for &g in &board.edge_faces[e as usize] {
                    if g != u32::MAX && distance[g as usize] == u32::MAX {
                        distance[g as usize] = distance[f as usize] + 1;
                        queue.push_back(g);
                    }
                }
            }
        }
        distance
    }

    fn random_region_loop(&mut self, board: &Board) -> (Vec<u32>, usize) {
        let mut inside = vec![false; board.num_faces];
        let mut outside = vec![false; board.num_faces];
        let mut degree = vec![0u8; board.num_vertices];
        let mut in_frontier = vec![false; board.num_faces];
        let mut frontier: Vec<u32> = Vec::new();
        let mut loop_edges = 0usize;
        let is_loop = |inside: &[bool], e: usize| {
            board.edge_faces[e]
                .iter()
                .filter(|&&f| f != u32::MAX && inside[f as usize])
                .count()
                == 1
        };
        let start = (self.random() % board.num_faces as u64) as u32;
        let mut candidate = Some(start);
        let mut inside_count = 0;
        let mut refilled_at = None;
        loop {
            if let Some(f) = candidate {
                let f = f as usize;
                // Tentatively add the face, updating the loop's vertex degrees
                let edges = board.face_edges_of(f);
                let before = loop_edges;
                inside[f] = true;
                let mut valid = true;
                for &e in edges {
                    let now = is_loop(&inside, e as usize);
                    for &v in &board.edge_vertices[e as usize] {
                        if now {
                            degree[v as usize] += 1;
                        } else {
                            degree[v as usize] -= 1;
                        }
                    }
                    if now {
                        loop_edges += 1;
                    } else {
                        loop_edges -= 1;
                    }
                }
                for &e in edges {
                    for &v in &board.edge_vertices[e as usize] {
                        if degree[v as usize] != 0 && degree[v as usize] != 2 {
                            valid = false;
                        }
                    }
                }
                // A single loop: following it from one of the face's loop edges visits them all
                if valid {
                    match edges.iter().copied().find(|&e| is_loop(&inside, e as usize)) {
                        None => valid = false,
                        Some(first) => {
                            let mut count = 1;
                            let (mut previous, mut vertex) = (first, board.edge_vertices[first as usize][1]);
                            loop {
                                let next = board
                                    .vertex_edges_of(vertex as usize)
                                    .iter()
                                    .copied()
                                    .find(|&e| e != previous && is_loop(&inside, e as usize))
                                    .unwrap();
                                if next == first {
                                    break;
                                }
                                count += 1;
                                vertex = board.other_vertex(next, vertex);
                                previous = next;
                            }
                            valid = count == loop_edges;
                        }
                    }
                }
                if valid {
                    inside_count += 1;
                    for &e in edges {
                        for &g in &board.edge_faces[e as usize] {
                            if g != u32::MAX && !inside[g as usize] && !outside[g as usize] && !in_frontier[g as usize]
                            {
                                in_frontier[g as usize] = true;
                                frontier.push(g);
                            }
                        }
                    }
                } else {
                    // Undo
                    inside[f] = false;
                    for &e in edges {
                        let was = is_loop(&inside, e as usize);
                        for &v in &board.edge_vertices[e as usize] {
                            if was {
                                degree[v as usize] += 1;
                            } else {
                                degree[v as usize] -= 1;
                            }
                        }
                    }
                    loop_edges = before;
                }
            }
            if frontier.is_empty() {
                // Stop once trying again added nothing
                if refilled_at == Some(inside_count) {
                    break;
                }
                refilled_at = Some(inside_count);
                // Faces far from the loop, and how far each face is from them
                let touching = (0..board.num_faces as u32).filter(|&f| {
                    board
                        .face_edges_of(f as usize)
                        .iter()
                        .any(|&e| is_loop(&inside, e as usize))
                });
                let loop_distance = Self::face_distances(board, touching);
                let far = (0..board.num_faces as u32).filter(|&f| loop_distance[f as usize] > LOOP_MAX_GAP);
                let far_distance = Self::face_distances(board, far);
                for f in 0..board.num_faces {
                    if !inside[f]
                        && far_distance[f] <= LOOP_MAX_GAP + 1
                        && board.face_edges_of(f).iter().any(|&e| is_loop(&inside, e as usize))
                    {
                        outside[f] = false;
                        in_frontier[f] = true;
                        frontier.push(f as u32);
                    }
                }
                if frontier.is_empty() {
                    break;
                }
            }
            let index = (self.random() % frontier.len() as u64) as usize;
            let f = frontier.swap_remove(index);
            in_frontier[f as usize] = false;
            if self.random().is_multiple_of(LOOP_OUTSIDE_ODDS) {
                outside[f as usize] = true;
                candidate = None;
            } else {
                candidate = (!inside[f as usize]).then_some(f);
            }
        }
        (
            (0..board.num_edges as u32)
                .filter(|&e| is_loop(&inside, e as usize))
                .collect(),
            inside_count,
        )
    }

    /// Does a bounded amount of work (about one face's worth)
    pub fn step(&mut self, board: &Board, topology: Option<&Topology>, rules: Rules) -> Status {
        match std::mem::replace(&mut self.phase, Phase::Failed) {
            Phase::Additive {
                order,
                position,
                attempts,
            } => self.additive_step(board, topology, rules, order, position, attempts),
            Phase::Guided { attempts } => self.guided_step(board, topology, rules, attempts),
            Phase::Minimize { order, position } => self.minimize_step(board, topology, rules, order, position),
            Phase::Done => {
                self.phase = Phase::Done;
                Status::Done
            }
            Phase::Failed => Status::Failed,
        }
    }

    // Adds one value. With two known solutions, it's the value one of them has on a face where they differ, which rules
    // out the other (and keeps a solution); otherwise a random value on the next face in the random order.
    fn additive_step(
        &mut self,
        board: &Board,
        topology: Option<&Topology>,
        rules: Rules,
        order: Vec<u32>,
        position: usize,
        attempts: u32,
    ) -> Status {
        if let Some((additions, kept)) = self.witness_values(board) {
            for &(face, value) in &additions {
                self.face_values[face] = value;
            }
            // The kept solution is still a solution: is there another?
            let values = self.face_values.clone();
            let others = {
                let _timer = profile::Timer::new(0);
                let counter = self.counter.get_or_insert_with(|| SolutionCounter::new(board));
                let others = counter.solutions(board, &values, 1, 10000, Some(&kept), WITNESS_ROUNDS);
                (others, counter.witness_model.take())
            };
            match others {
                (Some(_), Some(model)) => {
                    // No other solution found quickly: the last model (not a solution, but different) is the next witness
                    for &(face, value) in &additions {
                        self.emit(EVENT_DEFINE, face as u32, value);
                    }
                    self.witnesses = Some((kept, model));
                    self.second_witness_is_solution = false;
                    self.phase = Phase::Additive {
                        order,
                        position,
                        attempts,
                    };
                    return Status::Running;
                }
                (Some(others), None) => {
                    for &(face, value) in &additions {
                        self.emit(EVENT_DEFINE, face as u32, value);
                    }
                    for other in &others {
                        self.remember(board, other);
                    }
                    let mut solutions = vec![kept];
                    solutions.extend(others);
                    return self.after_count(board, topology, rules, solutions, order, position, attempts);
                }
                (None, _) => {
                    // (the SAT solver gave up)
                    for &(face, _) in &additions {
                        self.face_values[face] = NO_VALUE;
                    }
                    self.witnesses = None;
                }
            }
        }

        if position >= order.len() {
            // A whole pass without a unique solution: start over (generateFaceAdditive's retry)
            self.start_additive(board, attempts + 1);
            return if matches!(self.phase, Phase::Failed) {
                Status::Failed
            } else {
                Status::Running
            };
        }
        let face = order[position] as usize;
        if self.face_values[face] != NO_VALUE {
            self.phase = Phase::Additive {
                order,
                position: position + 1,
                attempts,
            };
            return Status::Running;
        }
        let n = board.face_edge_count(face) as u32;
        // A random value, not all of the face's edges (the shuffle is redone once if it starts with 0, which makes 0s
        // less likely)
        let mut values: Vec<u32> = (0..n).collect();
        self.shuffle(&mut values);
        if values[0] == 0 {
            self.shuffle(&mut values);
        }
        for value in values {
            self.face_values[face] = value as u8;
            // (None: too many SAT iterations, treated as no solution)
            let solutions = self.solution_count(board).unwrap_or_default();
            if !solutions.is_empty() {
                self.emit(EVENT_DEFINE, face as u32, value as u8);
                return self.after_count(board, topology, rules, solutions, order, position + 1, attempts);
            }
        }
        self.face_values[face] = NO_VALUE;
        self.phase = Phase::Additive {
            order,
            position: position + 1,
            attempts,
        };
        Status::Running
    }

    // Adds values from the known solution on faces next to edges the sound rules can't decide yet (a fraction of them at
    // once), until they solve the puzzle, which proves it's unique (like Loopy's generator, which has no SAT solver).
    fn guided_step(&mut self, board: &Board, topology: Option<&Topology>, rules: Rules, attempts: u32) -> Status {
        let deduction = {
            let _timer = profile::Timer::new(5);
            cansolve::deduce(board, rules.sound.unwrap(), &self.face_values, true)
        };
        let Ok(deduction) = deduction else {
            // (can't happen: the solution satisfies the values)
            self.emit(EVENT_RESET, 0, 0);
            self.start_additive(board, attempts + 1);
            return Status::Running;
        };
        if deduction.solved {
            return self.finish_additive(board, topology, rules);
        }
        let counts = Self::face_counts(board, &self.solution);
        let mut candidates: Vec<u32> = (0..board.num_faces as u32)
            .filter(|&f| {
                let f = f as usize;
                self.face_values[f] == NO_VALUE
                    && (counts[f] as usize) < board.face_edge_count(f)
                    && board
                        .face_edges_of(f)
                        .iter()
                        .any(|&e| deduction.edge_state[e as usize] == cansolve::WHITE)
            })
            .collect();
        if candidates.is_empty() {
            // Undecided edges only next to faces that can't take a value: give up on this solution
            self.emit(EVENT_RESET, 0, 0);
            self.start_additive(board, attempts + 1);
            return Status::Running;
        }
        self.shuffle(&mut candidates);
        let count = candidates.len().div_ceil(GUIDED_DIVISOR);
        for &f in &candidates[..count] {
            let value = counts[f as usize];
            self.face_values[f as usize] = value;
            self.emit(EVENT_DEFINE, f, value);
        }
        self.phase = Phase::Guided { attempts };
        Status::Running
    }

    #[allow(clippy::too_many_arguments)]
    fn after_count(
        &mut self,
        board: &Board,
        topology: Option<&Topology>,
        rules: Rules,
        mut solutions: Vec<Vec<u32>>,
        order: Vec<u32>,
        position: usize,
        attempts: u32,
    ) -> Status {
        if solutions.len() == 1 {
            self.solution = solutions.pop().unwrap();
            self.witnesses = None;
            return self.finish_additive(board, topology, rules);
        }
        let b = solutions.pop().unwrap();
        let a = solutions.pop().unwrap();
        if rules.sound.is_some() {
            // From here, values come from one solution until the sound rules solve it (no more SAT)
            self.solution = if self.random() & 1 == 0 { a } else { b };
            self.phase = Phase::Guided { attempts };
            return Status::Running;
        }
        self.witnesses = Some((a, b));
        self.second_witness_is_solution = true;
        self.phase = Phase::Additive {
            order,
            position,
            attempts,
        };
        Status::Running
    }

    // Values to add from one of the two witness solutions (chosen randomly) on faces where their counts differ (not all
    // of the face's edges): a random quarter of those faces (at least one), so a single count can rule out more than the
    // other witness. Returns them and the solution they come from.
    #[allow(clippy::type_complexity)]
    fn witness_values(&mut self, board: &Board) -> Option<(Vec<(usize, u8)>, Vec<u32>)> {
        let (a, b) = self.witnesses.take()?;
        let counts_a = Self::face_counts(board, &a);
        let counts_b = Self::face_counts(board, &b);
        // (only a solution can be kept)
        let keep_a = !self.second_witness_is_solution || self.random() & 1 == 0;
        let options: &[bool] = if self.second_witness_is_solution {
            &[keep_a, !keep_a]
        } else {
            &[true]
        };
        for &keep_first in options {
            let (kept, other) = if keep_first {
                (&counts_a, &counts_b)
            } else {
                (&counts_b, &counts_a)
            };
            let mut candidates: Vec<u32> = (0..board.num_faces as u32)
                .filter(|&f| {
                    let f = f as usize;
                    self.face_values[f] == NO_VALUE
                        && kept[f] != other[f]
                        && (kept[f] as usize) < board.face_edge_count(f)
                })
                .collect();
            if candidates.is_empty() {
                continue;
            }
            self.shuffle(&mut candidates);
            let count = candidates.len().div_ceil(WITNESS_DIVISOR).min(MAX_WITNESS_VALUES);
            let additions = candidates[..count]
                .iter()
                .map(|&f| (f as usize, kept[f as usize]))
                .collect();
            return Some((additions, if keep_first { a } else { b }));
        }
        None
    }

    fn face_counts(board: &Board, edges: &[u32]) -> Vec<u8> {
        let mut counts = vec![0u8; board.num_faces];
        for &e in edges {
            for &f in &board.edge_faces[e as usize] {
                if f != u32::MAX {
                    counts[f as usize] += 1;
                }
            }
        }
        counts
    }

    // Unique: fill the faces (with a difficulty check), check the result can be minimized, and start minimizing
    fn finish_additive(&mut self, board: &Board, topology: Option<&Topology>, rules: Rules) -> Status {
        // (Loop-first values already solve it: minimizing starts from them rather than every face's value, which would
        // take about twice as many checks)
        if self.check != Check::None && !self.loop_first {
            let counts = self.solution_face_counts(board);
            for (f, &count) in counts.iter().enumerate() {
                if self.face_values[f] == NO_VALUE {
                    self.face_values[f] = count;
                    self.emit(EVENT_DEFINE, f as u32, count);
                }
            }
        }
        // No face with all of its edges in the loop (those make boring puzzles), and solvable at the difficulty
        let full = (0..board.num_faces).any(|f| self.face_values[f] as usize == board.face_edge_count(f));
        let values = self.face_values.clone();
        if full || !self.can_solve(board, topology, rules, &values) {
            self.emit(EVENT_RESET, 0, 0);
            self.start_additive(board, 1);
            return Status::Running;
        }
        let mut order: Vec<u32> = (0..board.num_faces as u32).collect();
        self.shuffle(&mut order);
        self.start_minimize(order);
        Status::Running
    }

    fn start_minimize(&mut self, order: Vec<u32>) {
        self.minimize_start_values = self.face_values.clone();
        self.kept.fill(false);
        self.uniqueness = None;
        self.block = 1;
        self.checkpoint = None;
        self.cluster_ends.clear();
        if self.loop_first {
            // Consecutive faces in the random order (spread across the board: clusters of nearby faces would each be
            // emptied while the faces around them still had values, leaving large areas without values)
            self.cluster_ends
                .extend((MINIMIZE_CLUSTER_SIZE..order.len()).step_by(MINIMIZE_CLUSTER_SIZE));
        }
        self.cluster_ends.push(order.len());
        self.phase = Phase::Minimize { order, position: 0 };
    }

    // Removes values in blocks: a block whose values can all be removed (still unique, and solvable at the difficulty) is
    // removed together and the next block is twice as big; otherwise the block is halved and retried, down to single
    // values, which are kept if they can't be removed. This needs far fewer checks than one value at a time while most
    // values can be removed.
    fn minimize_step(
        &mut self,
        board: &Board,
        topology: Option<&Topology>,
        rules: Rules,
        order: Vec<u32>,
        position: usize,
    ) -> Status {
        if position >= order.len() {
            self.phase = Phase::Done;
            return Status::Done;
        }
        // Skip faces without values
        let face = order[position] as usize;
        if self.face_values[face] == NO_VALUE {
            self.emit(EVENT_MINIMIZE, face as u32, NO_VALUE);
            self.phase = Phase::Minimize {
                order,
                position: position + 1,
            };
            return Status::Running;
        }

        let cluster = self.cluster_ends.partition_point(|&end| end <= position);
        let cluster_end = self.cluster_ends[cluster];
        if self.loop_first
            && let Some(sound) = rules.sound
            && self.checkpoint.as_ref().is_none_or(|(c, _)| *c != cluster)
        {
            let cluster_start = if cluster == 0 {
                0
            } else {
                self.cluster_ends[cluster - 1]
            };
            let mut faces = vec![false; board.num_faces];
            for &f in &order[cluster_start..cluster_end] {
                faces[f as usize] = true;
            }
            let _timer = profile::Timer::new(5);
            self.checkpoint = cansolve::checkpoint(board, sound, &self.face_values, &faces, true).map(|c| (cluster, c));
        }
        let block_end = (position + self.block).min(cluster_end);
        let block: Vec<usize> = order[position..block_end]
            .iter()
            .map(|&f| f as usize)
            .filter(|&f| self.face_values[f] != NO_VALUE)
            .collect();
        let saved: Vec<u8> = block.iter().map(|&f| self.face_values[f]).collect();
        for &f in &block {
            self.face_values[f] = NO_VALUE;
        }
        let values = self.face_values.clone();
        let removable = self.removable(board, topology, rules, &values);

        if removable {
            let end = block_end;
            for &f in &order[position..end] {
                self.emit(EVENT_MINIMIZE, f, NO_VALUE);
            }
            self.block = (self.block * 2).min(MAX_MINIMIZE_BLOCK);
            self.phase = Phase::Minimize { order, position: end };
            return Status::Running;
        }

        for (&f, &value) in block.iter().zip(&saved) {
            self.face_values[f] = value;
        }
        if self.block > 1 {
            self.block /= 2;
            self.phase = Phase::Minimize { order, position };
        } else {
            self.kept[face] = true;
            self.emit(EVENT_MINIMIZE, face as u32, saved[0]);
            self.phase = Phase::Minimize {
                order,
                position: position + 1,
            };
        }
        Status::Running
    }

    // Whether these values (after removing some) are still unique and solvable at the difficulty. Cheapest first: known
    // loops showing multiple solutions; the sound rules solving it (which proves it unique, and maybe solvable at the
    // difficulty); the difficulty check; then SAT uniqueness.
    fn removable(&mut self, board: &Board, topology: Option<&Topology>, rules: Rules, values: &[u8]) -> bool {
        if self.known_multiple(values) {
            return false;
        }
        if let Some(sound) = rules.sound {
            let solved = {
                let _timer = profile::Timer::new(5);
                let deduction = match &self.checkpoint {
                    Some((_, checkpoint)) => cansolve::deduce_from(checkpoint, board, sound, values),
                    None => cansolve::deduce(board, sound, values, true),
                };
                matches!(deduction, Ok(d) if d.solved)
            };
            if solved {
                return rules.sound_implies_difficulty || self.can_solve(board, topology, rules, values);
            }
            if rules.sound_only {
                return false;
            }
        }
        self.can_solve(board, topology, rules, values) && self.is_unique(board, values)
    }

    fn can_solve(&self, board: &Board, topology: Option<&Topology>, rules: Rules, values: &[u8]) -> bool {
        let _timer = profile::Timer::new(1);
        if self.check == Check::None {
            return true;
        }
        // Sound-only: the sound rules are the check (a subset of the difficulty's rules, so stricter), and the main and
        // easier rules aren't needed (or loaded)
        if rules.sound_only
            && rules.sound_implies_difficulty
            && let Some(sound) = rules.sound
        {
            return matches!(cansolve::deduce(board, sound, values, true), Ok(d) if d.solved);
        }
        // An easier difficulty's rules are a subset of this one's, so if they solve it, so would these (and they're
        // much cheaper)
        if let Some(easier) = rules.easier
            && matches!(cansolve::can_solve(board, easier, values), Ok(true))
        {
            profile::count(3, 0);
            return true;
        }
        match self.check {
            Check::None => true,
            Check::Simple => matches!(cansolve::can_solve(board, rules.main.unwrap(), values), Ok(true)),
            Check::Hard => matches!(
                engine::can_solve_hard(topology.unwrap(), rules.main.unwrap(), values),
                Ok(true)
            ),
        }
    }

    // The number of solutions of the current values (0, 1, or 2 for more), and the solution if unique. None if the
    // SAT solver gave up.
    // The solutions of the current values: none, one (unique), or two (witnesses of multiple solutions), or None if the
    // SAT solver gave up
    fn solution_count(&mut self, board: &Board) -> Option<Vec<Vec<u32>>> {
        if let Some((a, b)) = self.known_pair(&self.face_values) {
            return Some(vec![self.known[a].1.clone(), self.known[b].1.clone()]);
        }
        let values = self.face_values.clone();
        self.solutions(board, &values)
    }

    // Whether the known solution is the only one (false if the SAT solver gave up), with the minimization's incremental
    // uniqueness checker
    fn is_unique(&mut self, board: &Board, values: &[u8]) -> bool {
        let _timer = profile::Timer::new(4);
        if self.uniqueness.is_none() {
            self.uniqueness = Some(Uniqueness::new(board, &self.minimize_start_values, &self.solution));
        }
        let active: Vec<bool> = values.iter().map(|&v| v != NO_VALUE).collect();
        let kept = self.kept.clone();
        matches!(
            self.uniqueness
                .as_mut()
                .unwrap()
                .is_unique(board, &active, &kept, UNIQUENESS_ROUNDS),
            Some(true)
        )
    }

    // Up to two solutions (remembering them), or None if the SAT solver gave up
    fn solutions(&mut self, board: &Board, values: &[u8]) -> Option<Vec<Vec<u32>>> {
        let _timer = profile::Timer::new(0);
        let counter = self.counter.get_or_insert_with(|| SolutionCounter::new(board));
        let solutions = counter.solutions(board, values, 2, 10000, None, u64::MAX)?;
        for solution in &solutions {
            self.remember(board, solution);
        }
        Some(solutions)
    }

    fn remember(&mut self, board: &Board, solution: &[u32]) {
        let mut edges = solution.to_vec();
        edges.sort_unstable();
        if self.known.iter().any(|(_, known)| *known == edges) {
            return;
        }
        let mut counts = vec![0u8; board.num_faces];
        for &e in &edges {
            for &f in &board.edge_faces[e as usize] {
                if f != u32::MAX {
                    counts[f as usize] += 1;
                }
            }
        }
        self.known.push((counts, edges));
        if self.known.len() > MAX_KNOWN_SOLUTIONS {
            self.known.remove(0);
        }
    }

    // Whether two known loops are solutions for these values (so there are multiple solutions)
    fn known_multiple(&self, values: &[u8]) -> bool {
        self.known_pair(values).is_some()
    }

    // Two known loops that are solutions for these values (indices into known), if any
    fn known_pair(&self, values: &[u8]) -> Option<(usize, usize)> {
        let _timer = profile::Timer::new(2);
        if values.iter().all(|&v| v == NO_VALUE) {
            return None;
        }
        let mut first = None;
        for (index, (counts, _)) in self.known.iter().enumerate().rev() {
            let mut valid = true;
            let mut touches = false;
            for (f, &value) in values.iter().enumerate() {
                if value != NO_VALUE {
                    if counts[f] != value {
                        valid = false;
                        break;
                    }
                    touches |= counts[f] > 0;
                }
            }
            if valid && touches {
                match first {
                    None => first = Some(index),
                    Some(a) => return Some((a, index)),
                }
            }
        }
        None
    }

    fn solution_face_counts(&self, board: &Board) -> Vec<u8> {
        let mut counts = vec![0u8; board.num_faces];
        for &e in &self.solution {
            for &f in &board.edge_faces[e as usize] {
                if f != u32::MAX {
                    counts[f as usize] += 1;
                }
            }
        }
        counts
    }

    /// The puzzle's face values and its solution's edges (once done)
    pub fn result(&self) -> (&[u8], &[u32]) {
        (&self.face_values, &self.solution)
    }
}

/// Time spent in SAT solves, difficulty checks and known-solution checks, in native builds with the "profile" feature
pub mod profile {
    pub const NAMES: [&str; 6] = [
        "SAT solves",
        "difficulty checks",
        "known solution checks",
        "-",
        "SAT solving (after building)",
        "-",
    ];

    #[cfg(feature = "profile")]
    pub static mut TOTALS: [f64; 6] = [0.0; 6];
    #[cfg(feature = "profile")]
    pub static mut COUNTS: [u64; 6] = [0; 6];

    #[inline]
    pub fn count(_kind: usize, _amount: u64) {
        #[cfg(feature = "profile")]
        unsafe {
            COUNTS[_kind] += 1
        }
    }

    pub struct Timer {
        #[cfg(feature = "profile")]
        kind: usize,
        #[cfg(feature = "profile")]
        start: std::time::Instant,
    }

    impl Timer {
        #[inline]
        pub fn new(_kind: usize) -> Timer {
            Timer {
                #[cfg(feature = "profile")]
                kind: _kind,
                #[cfg(feature = "profile")]
                start: std::time::Instant::now(),
            }
        }
    }

    #[cfg(feature = "profile")]
    impl Drop for Timer {
        fn drop(&mut self) {
            unsafe {
                TOTALS[self.kind] += self.start.elapsed().as_secs_f64();
                COUNTS[self.kind] += 1;
            }
        }
    }
}
