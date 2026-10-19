//! WebAssembly exports for the Slitherlink core. The JS side (src/wasm/slitherCore.ts) passes flat typed arrays through
//! linear memory: it allocates with `alloc`, writes, calls, reads, and frees with `dealloc`.

pub mod cansolve;
pub mod engine;
pub mod generator;
pub mod sat;
pub mod solver;
pub mod topology;

use solver::{Board, SolveResult};
use std::cell::RefCell;

thread_local! {
    static GENERATORS: RefCell<Vec<Option<generator::Generator>>> = const { RefCell::new(Vec::new()) };
    static TOPOLOGIES: RefCell<Vec<Option<topology::Topology>>> = const { RefCell::new(Vec::new()) };
    static RULE_SETS: RefCell<Vec<Option<cansolve::RuleSet>>> = const { RefCell::new(Vec::new()) };
    static BOARDS: RefCell<Vec<Option<Board>>> = const { RefCell::new(Vec::new()) };
    // Output of the last solve: [solution count, then for each solution its length and edge indices]
    static OUTPUT: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

#[unsafe(no_mangle)]
pub extern "C" fn alloc(bytes: usize) -> *mut u8 {
    let mut buffer = Vec::<u8>::with_capacity(bytes.max(1));
    let pointer = buffer.as_mut_ptr();
    std::mem::forget(buffer);
    pointer
}

/// # Safety
/// `pointer` must come from `alloc` with the same `bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(pointer: *mut u8, bytes: usize) {
    unsafe {
        drop(Vec::from_raw_parts(pointer, 0, bytes.max(1)));
    }
}

/// Registers a board, returning its handle.
///
/// # Safety
/// The pointers must reference `u32` arrays of the given lengths: `edge_vertices` (2 per edge), `face_edge_offsets`
/// (faces + 1, CSR offsets into `face_edges`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn board_create(
    num_vertices: u32,
    edge_vertices: *const u32,
    num_edges: u32,
    face_edge_offsets: *const u32,
    num_faces: u32,
    face_edges: *const u32,
    num_face_edges: u32,
) -> u32 {
    let board = unsafe {
        Board::new(
            num_vertices as usize,
            std::slice::from_raw_parts(edge_vertices, 2 * num_edges as usize),
            std::slice::from_raw_parts(face_edge_offsets, num_faces as usize + 1),
            std::slice::from_raw_parts(face_edges, num_face_edges as usize),
        )
    };
    BOARDS.with(|boards| {
        let mut boards = boards.borrow_mut();
        if let Some(index) = boards.iter().position(|b| b.is_none()) {
            boards[index] = Some(board);
            index as u32
        } else {
            boards.push(Some(board));
            (boards.len() - 1) as u32
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn board_dispose(handle: u32) {
    BOARDS.with(|boards| {
        if let Some(slot) = boards.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}

/// Finds up to `max_solutions` solutions. Returns the number found, or -1 if it gave up (more than `max_iterations`
/// SAT models, or `conflict_budget` conflicts). The solutions are
/// then available via `output_pointer`/`output_length`.
///
/// # Safety
/// `face_values` (i8, -1 for no value) and `edge_states` (u8: 0 white, 1 black, 2 red) must have one entry per face and
/// per edge of the board.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn solve(
    handle: u32,
    face_values: *const i8,
    edge_states: *const u8,
    max_solutions: u32,
    max_iterations: f64,
    conflict_budget: f64,
) -> i32 {
    BOARDS.with(|boards| {
        let boards = boards.borrow();
        let board = boards[handle as usize].as_ref().expect("disposed board");
        let (face_values, edge_states) = unsafe {
            (
                std::slice::from_raw_parts(face_values, board.num_faces),
                std::slice::from_raw_parts(edge_states, board.num_edges),
            )
        };
        let (result, conflicts) = solver::solve(
            board,
            face_values,
            edge_states,
            max_solutions as usize,
            max_iterations as u64,
            conflict_budget as u64,
        );
        unsafe { LAST_CONFLICTS = conflicts as f64 };
        OUTPUT.with(|output| {
            let mut output = output.borrow_mut();
            output.clear();
            match result {
                SolveResult::BudgetExceeded => -1,
                SolveResult::Solutions(solutions) => {
                    output.push(solutions.len() as u32);
                    for solution in &solutions {
                        output.push(solution.len() as u32);
                        output.extend_from_slice(solution);
                    }
                    solutions.len() as i32
                }
            }
        })
    })
}

static mut LAST_CONFLICTS: f64 = 0.0;

/// SAT conflicts in the last solve (for profiling)
#[unsafe(no_mangle)]
pub extern "C" fn last_conflicts() -> f64 {
    unsafe { LAST_CONFLICTS }
}

#[unsafe(no_mangle)]
pub extern "C" fn output_pointer() -> *const u32 {
    OUTPUT.with(|output| output.borrow().as_ptr())
}

#[unsafe(no_mangle)]
pub extern "C" fn output_length() -> u32 {
    OUTPUT.with(|output| output.borrow().len() as u32)
}

/// Registers compiled pattern rules (see compilePatternRules.ts) for a board, returning their handle.
///
/// # Safety
/// `clauses` must reference `clauses_length` u32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rules_create(
    board_handle: u32,
    num_sectors: u32,
    clauses: *const u32,
    clauses_length: u32,
    clause_count: u32,
) -> u32 {
    let num_edges = BOARDS.with(|boards| boards.borrow()[board_handle as usize].as_ref().unwrap().num_edges);
    let data = unsafe { std::slice::from_raw_parts(clauses, clauses_length as usize) };
    let rules = cansolve::RuleSet::new(num_edges, num_sectors as usize, data, clause_count as usize);
    RULE_SETS.with(|sets| {
        let mut sets = sets.borrow_mut();
        sets.push(Some(rules));
        (sets.len() - 1) as u32
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn rules_dispose(handle: u32) {
    RULE_SETS.with(|sets| {
        if let Some(slot) = sets.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}

/// Whether the rules solve the puzzle with these face values (one u8 per face, 0xff for none): 1 if so, 0 if not, -1 if
/// the deductions hit a contradiction.
///
/// # Safety
/// `face_values` must have one entry per face of the board.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn can_solve(board_handle: u32, rules_handle: u32, face_values: *const u8) -> i32 {
    BOARDS.with(|boards| {
        let boards = boards.borrow();
        let board = boards[board_handle as usize].as_ref().expect("disposed board");
        let face_values = unsafe { std::slice::from_raw_parts(face_values, board.num_faces) };
        RULE_SETS.with(|sets| {
            let sets = sets.borrow();
            let rules = sets[rules_handle as usize].as_ref().expect("disposed rules");
            match cansolve::can_solve(board, rules, face_values) {
                Ok(true) => 1,
                Ok(false) => 0,
                Err(_) => -1,
            }
        })
    })
}

/// Registers a board's half-edge topology (see topology.rs), returning its handle: per half-edge (start, end, edge,
/// face or 0xffffffff, next, reversed), each vertex's incoming half-edges (CSR), each face's half-edges (CSR).
///
/// # Safety
/// The pointers must reference u32 arrays of the given lengths.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn topology_create(
    num_vertices: u32,
    num_edges: u32,
    num_faces: u32,
    half_edges: *const u32,
    num_half_edges: u32,
    vertex_offsets: *const u32,
    vertex_incoming: *const u32,
    face_offsets: *const u32,
    face_half_edges: *const u32,
) -> u32 {
    let (nv, nf, nh) = (num_vertices as usize, num_faces as usize, num_half_edges as usize);
    let topology = unsafe {
        topology::Topology::new(
            nv,
            num_edges as usize,
            nf,
            std::slice::from_raw_parts(half_edges, 6 * nh),
            std::slice::from_raw_parts(vertex_offsets, nv + 1),
            std::slice::from_raw_parts(vertex_incoming, nh),
            std::slice::from_raw_parts(face_offsets, nf + 1),
            std::slice::from_raw_parts(face_half_edges, *face_offsets.add(nf) as usize),
        )
    };
    TOPOLOGIES.with(|topologies| {
        let mut topologies = topologies.borrow_mut();
        topologies.push(Some(topology));
        (topologies.len() - 1) as u32
    })
}

/// Whether the HARD rules solve the puzzle (see engine.rs): 1 if so, 0 if not, -1 on a contradiction.
///
/// # Safety
/// `face_values` must have one entry per face.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn can_solve_hard(topology_handle: u32, rules_handle: u32, face_values: *const u8) -> i32 {
    TOPOLOGIES.with(|topologies| {
        let topologies = topologies.borrow();
        let topology = topologies[topology_handle as usize]
            .as_ref()
            .expect("disposed topology");
        let face_values = unsafe { std::slice::from_raw_parts(face_values, topology.num_faces) };
        RULE_SETS.with(|sets| {
            let sets = sets.borrow();
            let rules = sets[rules_handle as usize].as_ref().expect("disposed rules");
            match engine::can_solve_hard(topology, rules, face_values) {
                Ok(true) => 1,
                Ok(false) => 0,
                Err(_) => -1,
            }
        })
    })
}

fn with_generator_context<T>(
    board_handle: u32,
    topology_handle: u32,
    rules_handle: u32,
    callback: impl FnOnce(&solver::Board, Option<&topology::Topology>, Option<&cansolve::RuleSet>) -> T,
) -> T {
    BOARDS.with(|boards| {
        let boards = boards.borrow();
        let board = boards[board_handle as usize].as_ref().expect("disposed board");
        TOPOLOGIES.with(|topologies| {
            let topologies = topologies.borrow();
            let topology = topologies.get(topology_handle as usize).and_then(|t| t.as_ref());
            RULE_SETS.with(|sets| {
                let sets = sets.borrow();
                let rules = sets.get(rules_handle as usize).and_then(|r| r.as_ref());
                callback(board, topology, rules)
            })
        })
    })
}

/// Starts generating a puzzle (see generator.rs), returning its handle. `check` is 0 for no difficulty check, 1 for the
/// EASY/MEDIUM rules and 2 for HARD's (with the rules handle, and for HARD the topology handle; 0xffffffff for none).
/// `resume` holds earlier progress events (type, face, value) to continue from.
///
/// # Safety
/// `resume` must reference `resume_length` u32s.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn gen_create(
    board_handle: u32,
    topology_handle: u32,
    rules_handle: u32,
    check: u32,
    seed_low: u32,
    seed_high: u32,
    resume: *const u32,
    resume_length: u32,
) -> u32 {
    let resume = if resume_length == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(resume, resume_length as usize) }
    };
    let check = match check {
        1 => generator::Check::Simple,
        2 => generator::Check::Hard,
        _ => generator::Check::None,
    };
    let seed = ((seed_high as u64) << 32) | seed_low as u64;
    let generator = with_generator_context(board_handle, topology_handle, rules_handle, |board, topology, rules| {
        generator::Generator::new(board, topology, rules, check, seed, resume)
    });
    GENERATORS.with(|generators| {
        let mut generators = generators.borrow_mut();
        if let Some(index) = generators.iter().position(|g| g.is_none()) {
            generators[index] = Some(generator);
            index as u32
        } else {
            generators.push(Some(generator));
            (generators.len() - 1) as u32
        }
    })
}

/// Runs up to `steps` steps (about a face each). Returns 0 while running, 1 when done, 2 if generation failed. The
/// progress events so far are then in the output buffer (type, face, value triples).
#[unsafe(no_mangle)]
pub extern "C" fn gen_step(handle: u32, board_handle: u32, topology_handle: u32, rules_handle: u32, steps: u32) -> u32 {
    GENERATORS.with(|generators| {
        let mut generators = generators.borrow_mut();
        let generator = generators[handle as usize].as_mut().expect("disposed generator");
        let mut status = generator::Status::Running;
        with_generator_context(board_handle, topology_handle, rules_handle, |board, topology, rules| {
            for _ in 0..steps.max(1) {
                status = generator.step(board, topology, rules);
                if !matches!(status, generator::Status::Running) {
                    break;
                }
            }
        });
        OUTPUT.with(|output| {
            let mut output = output.borrow_mut();
            output.clear();
            output.append(&mut generator.events);
        });
        match status {
            generator::Status::Running => 0,
            generator::Status::Done => 1,
            generator::Status::Failed => 2,
        }
    })
}

/// Puts the generated puzzle in the output buffer: its face values (0xff for none), then its solution's edges
#[unsafe(no_mangle)]
pub extern "C" fn gen_result(handle: u32) {
    GENERATORS.with(|generators| {
        let generators = generators.borrow();
        let generator = generators[handle as usize].as_ref().expect("disposed generator");
        let (values, solution) = generator.result();
        OUTPUT.with(|output| {
            let mut output = output.borrow_mut();
            output.clear();
            output.extend(values.iter().map(|&v| v as u32));
            output.extend_from_slice(solution);
        });
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn gen_dispose(handle: u32) {
    GENERATORS.with(|generators| {
        if let Some(slot) = generators.borrow_mut().get_mut(handle as usize) {
            *slot = None;
        }
    });
}
