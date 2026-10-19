import { TBoard } from '../board/core/TBoard.ts';
import { TEdge } from '../board/core/TEdge.ts';
import { TState } from '../data/core/TState.ts';
import { TEdgeStateData } from '../data/edge-state/TEdgeStateData.ts';
import { TFaceValueData } from '../data/face-value/TFaceValueData.ts';
import { SatSolveOptions, satSolve } from './SATSolver.ts';

/**
 * Same as satSolve. (This reused one MiniSat solver per board for puzzle generation; the WebAssembly solver already keeps
 * each board's structure loaded, and needs no per-solve clauses.)
 */
export const incrementalSatSolve = (
  board: TBoard,
  state: TState<TEdgeStateData & TFaceValueData>,
  options: SatSolveOptions,
): TEdge[][] => {
  return satSolve(board, state, options);
};
