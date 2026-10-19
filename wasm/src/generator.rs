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
use crate::solver::{self, Board, SolveResult, WHITE};
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
// Give up on an additive pass after this many (as in generateFaceAdditive.ts)
const MAX_ADDITIVE_ATTEMPTS: u32 = 100;

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
    Minimize {
        order: Vec<u32>,
        position: usize,
    },
    Done,
    Failed,
}

pub struct Generator {
    check: Check,
    rng: u64,
    face_values: Vec<u8>,
    // The solution's edges (once the puzzle is unique)
    solution: Vec<u32>,
    phase: Phase,
    pub events: Vec<u32>,
    // Loops found by SAT solves: each loop's number of edges around each face, and its sorted edges
    known: Vec<(Vec<u8>, Vec<u32>)>,
}

impl Generator {
    /// Starts generating, optionally resuming from earlier progress events (type, face, value triples)
    pub fn new(
        board: &Board,
        topology: Option<&Topology>,
        rules: Option<&RuleSet>,
        check: Check,
        seed: u64,
        resume: &[u32],
    ) -> Generator {
        let mut generator = Generator {
            check,
            rng: seed | 1,
            face_values: vec![NO_VALUE; board.num_faces],
            solution: Vec::new(),
            phase: Phase::Failed,
            events: Vec::new(),
            known: Vec::new(),
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
    fn resume(&mut self, board: &Board, topology: Option<&Topology>, rules: Option<&RuleSet>, events: &[u32]) {
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
                self.phase = Phase::Minimize { order, position: 0 };
                return;
            }
            // Couldn't resume
            self.emit(EVENT_RESET, 0, 0);
            defined.fill(NO_VALUE);
        }

        if defined.iter().any(|&v| v != NO_VALUE) {
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
        let mut order: Vec<u32> = (0..board.num_faces as u32).collect();
        self.shuffle(&mut order);
        self.phase = Phase::Additive {
            order,
            position: 0,
            attempts,
        };
    }

    /// Does a bounded amount of work (about one face's worth)
    pub fn step(&mut self, board: &Board, topology: Option<&Topology>, rules: Option<&RuleSet>) -> Status {
        match std::mem::replace(&mut self.phase, Phase::Failed) {
            Phase::Additive {
                order,
                position,
                attempts,
            } => self.additive_step(board, topology, rules, order, position, attempts),
            Phase::Minimize { order, position } => self.minimize_step(board, topology, rules, order, position),
            Phase::Done => {
                self.phase = Phase::Done;
                Status::Done
            }
            Phase::Failed => Status::Failed,
        }
    }

    fn additive_step(
        &mut self,
        board: &Board,
        topology: Option<&Topology>,
        rules: Option<&RuleSet>,
        order: Vec<u32>,
        position: usize,
        attempts: u32,
    ) -> Status {
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
            let count = match self.solution_count(board) {
                Some((count, solution)) => {
                    if count == 1 {
                        self.solution = solution;
                    }
                    count
                }
                // Too many SAT iterations: treated as no solution
                None => 0,
            };
            if count >= 1 {
                self.emit(EVENT_DEFINE, face as u32, value as u8);
                if count == 1 {
                    return self.finish_additive(board, topology, rules);
                }
                self.phase = Phase::Additive {
                    order,
                    position: position + 1,
                    attempts,
                };
                return Status::Running;
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

    // Unique: fill the faces (with a difficulty check), check the result can be minimized, and start minimizing
    fn finish_additive(&mut self, board: &Board, topology: Option<&Topology>, rules: Option<&RuleSet>) -> Status {
        if self.check != Check::None {
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
        self.phase = Phase::Minimize { order, position: 0 };
        Status::Running
    }

    fn minimize_step(
        &mut self,
        board: &Board,
        topology: Option<&Topology>,
        rules: Option<&RuleSet>,
        order: Vec<u32>,
        position: usize,
    ) -> Status {
        if position >= order.len() {
            self.phase = Phase::Done;
            return Status::Done;
        }
        let face = order[position] as usize;
        let value = self.face_values[face];
        if value == NO_VALUE {
            self.emit(EVENT_MINIMIZE, face as u32, NO_VALUE);
        } else {
            self.face_values[face] = NO_VALUE;
            let unique = matches!(self.solution_count(board), Some((1, _)));
            let values = self.face_values.clone();
            if unique && self.can_solve(board, topology, rules, &values) {
                self.emit(EVENT_MINIMIZE, face as u32, NO_VALUE);
            } else {
                self.face_values[face] = value;
                self.emit(EVENT_MINIMIZE, face as u32, value);
            }
        }
        self.phase = Phase::Minimize {
            order,
            position: position + 1,
        };
        Status::Running
    }

    fn can_solve(&self, board: &Board, topology: Option<&Topology>, rules: Option<&RuleSet>, values: &[u8]) -> bool {
        let _timer = profile::Timer::new(1);
        match self.check {
            Check::None => true,
            Check::Simple => matches!(cansolve::can_solve(board, rules.unwrap(), values), Ok(true)),
            Check::Hard => matches!(
                engine::can_solve_hard(topology.unwrap(), rules.unwrap(), values),
                Ok(true)
            ),
        }
    }

    // The number of solutions of the current values (0, 1, or 2 for more), and the solution if unique. None if the
    // SAT solver gave up.
    fn solution_count(&mut self, board: &Board) -> Option<(usize, Vec<u32>)> {
        if self.known_multiple(&self.face_values) {
            return Some((2, Vec::new()));
        }
        let values = self.face_values.clone();
        let solutions = self.solutions(board, &values)?;
        let count = solutions.len();
        let solution = if count == 1 { solutions[0].clone() } else { Vec::new() };
        Some((count, solution))
    }

    // Up to two solutions (remembering them), or None if the SAT solver gave up
    fn solutions(&mut self, board: &Board, values: &[u8]) -> Option<Vec<Vec<u32>>> {
        let _timer = profile::Timer::new(0);
        let face_values: Vec<i8> = values
            .iter()
            .map(|&v| if v == NO_VALUE { -1 } else { v as i8 })
            .collect();
        let edge_states = vec![WHITE; board.num_edges];
        match solver::solve(board, &face_values, &edge_states, 2, 10000, u64::MAX).0 {
            SolveResult::Solutions(solutions) => {
                for solution in &solutions {
                    self.remember(board, solution);
                }
                Some(solutions)
            }
            SolveResult::BudgetExceeded => None,
        }
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
        let _timer = profile::Timer::new(2);
        if values.iter().all(|&v| v == NO_VALUE) {
            return false;
        }
        let mut count = 0;
        for (counts, _) in self.known.iter().rev() {
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
                count += 1;
                if count >= 2 {
                    return true;
                }
            }
        }
        false
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
