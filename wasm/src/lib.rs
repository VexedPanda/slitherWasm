//! WebAssembly exports for the Slitherlink core. The JS side (src/wasm/slitherCore.ts) passes flat typed arrays through
//! linear memory: it allocates with `alloc`, writes, calls, reads, and frees with `dealloc`.

pub mod sat;
pub mod solver;

use solver::{Board, SolveResult};
use std::cell::RefCell;

thread_local! {
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
