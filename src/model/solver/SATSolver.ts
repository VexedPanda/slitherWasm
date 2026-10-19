/**
 * Solving puzzles (finding the loops that satisfy the clues), with the WebAssembly solver (src/wasm/slitherCore.ts).
 *
 * The original MiniSat-based solver is in MiniSatSolver.ts (only used to check this one).
 */
import { BUDGET_EXCEEDED, wasmSolve } from '../../wasm/slitherCore.ts';
import { TBoard } from '../board/core/TBoard.ts';
import { TEdge } from '../board/core/TEdge.ts';
import { TStructure } from '../board/core/TStructure.ts';
import { CompleteData } from '../data/combined/CompleteData.ts';
import { TCompleteData } from '../data/combined/TCompleteData.ts';
import { TState } from '../data/core/TState.ts';
import { TEdgeStateData } from '../data/edge-state/TEdgeStateData.ts';
import { TFaceValueData } from '../data/face-value/TFaceValueData.ts';
import { simpleRegionIsSolved } from '../data/simple-region/TSimpleRegionData.ts';
import { getSolvedPuzzle } from '../generator/getSolvedPuzzle.ts';
import { TSolvablePropertyPuzzle } from '../puzzle/TPuzzle.ts';
import { MaximumSolverIterationsError } from './errors/MaximumSolverIterationsError.ts';
import { MultipleSolutionsError } from './errors/MultipleSolutionsError.ts';

import { Property } from 'scenerystack/axon';

import assert, { assertEnabled } from '../../workarounds/assert.ts';

export const getSolvablePropertyPuzzle = <
  Structure extends TStructure = TStructure,
  Data extends TCompleteData = TCompleteData,
>(
  board: TBoard<Structure>,
  currentState: TState<Data>,
): TSolvablePropertyPuzzle<Structure, Data> | null => {
  try {
    // SAT solver doesn't work with fully solved boards, check this first
    if (simpleRegionIsSolved(currentState)) {
      return {
        board,
        stateProperty: new Property(currentState),
        solution: getSolvedPuzzle(board, currentState, []),
      };
    }

    // The solution must only depend on the clues, not on anything the player has marked (e.g. a restored puzzle with
    // a mistaken line would otherwise get that line baked into its solution, leaving the rest of the solution
    // unresolved, so correct marks would be highlighted as incorrect).
    const cleanState = CompleteData.fromFaces(board, (face) =>
      currentState.getFaceValue(face),
    ) as unknown as TState<Data>;

    const solutions = satSolve(board, cleanState, {
      maxIterations: 10000,
      failOnMultipleSolutions: true,
    });

    if (solutions.length !== 1) {
      return null;
    }

    const solution = getSolvedPuzzle(board, cleanState, solutions[0]);
    assertEnabled() && assert(simpleRegionIsSolved(solution.solvedState));

    return {
      board,
      stateProperty: new Property(currentState),
      solution: solution,
    };
  } catch (e) {
    if (e instanceof MultipleSolutionsError) {
      return null;
    } else {
      throw e;
    }
  }
};

export type SatSolveOptions = {
  maxIterations: number;
  failOnMultipleSolutions: boolean;
};

/**
 * Finds the solutions (each the edges of a single loop) for the state's face values and black/red edges, with the
 * WebAssembly solver (src/wasm/slitherCore.ts). With failOnMultipleSolutions, throws MultipleSolutionsError once a second
 * solution is found. Throws MaximumSolverIterationsError if it takes more than maxIterations SAT models (as the
 * MiniSat-based solver did).
 */
export const satSolve = (
  board: TBoard,
  state: TState<TEdgeStateData & TFaceValueData>,
  options: SatSolveOptions,
): TEdge[][] => {
  // (a u32 in the core; maxIterations is what bounds it)
  const maxSolutions = options.failOnMultipleSolutions ? 2 : 0xffffffff;
  const solutions = wasmSolve(board, state, maxSolutions, options.maxIterations, MAX_CONFLICTS);

  if (solutions === BUDGET_EXCEEDED) {
    throw new MaximumSolverIterationsError();
  }
  if (options.failOnMultipleSolutions && solutions.length > 1) {
    throw new MultipleSolutionsError(solutions);
  }
  return solutions;
};

// Effectively unlimited (MiniSat had no conflict limit either); the iteration limit is what bounds a solve
const MAX_CONFLICTS = 1e12;
