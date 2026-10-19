import { RuleGroupLimit, compilePatternRules } from '../../wasm/compilePatternRules.ts';
import { wasmCanSolve as coreCanSolve } from '../../wasm/slitherCore.ts';
import { TBoard } from '../board/core/TBoard.ts';
import { TCompleteData } from '../data/combined/TCompleteData.ts';
import { TAnnotatedAction } from '../data/core/TAnnotatedAction.ts';
import { TState } from '../data/core/TState.ts';
import { simpleRegionIsSolved } from '../data/simple-region/TSimpleRegionData.ts';
import { generalAllMixedGroup } from '../pattern/collection/generalAllMixedGroup.ts';
import { generalEdgeColorMixedGroup } from '../pattern/collection/generalEdgeColorMixedGroup.ts';
import { generalEdgeMixedGroup } from '../pattern/collection/generalEdgeMixedGroup.ts';
import { generalEdgeSectorMixedGroup } from '../pattern/collection/generalEdgeSectorMixedGroup.ts';
import { BoardPatternBoard } from '../pattern/pattern-board/BoardPatternBoard.ts';
import { BinaryPatternSolver } from '../solver/BinaryPatternSolver.ts';
import { CompositeSolver } from '../solver/CompositeSolver.ts';
import { FaceColorDisconnectionSolver } from '../solver/FaceColorDisconnectionSolver.ts';
import { SafeEdgeToFaceColorSolver } from '../solver/SafeEdgeToFaceColorSolver.ts';
import { SafeEdgeToSimpleRegionSolver } from '../solver/SafeEdgeToSimpleRegionSolver.ts';
import { SafeSolvedEdgeSolver } from '../solver/SafeSolvedEdgeSolver.ts';
import { SimpleFaceSolver } from '../solver/SimpleFaceSolver.ts';
import { SimpleLoopSolver } from '../solver/SimpleLoopSolver.ts';
import { SimpleVertexSolver } from '../solver/SimpleVertexSolver.ts';
import { iterateSolverFactory } from '../solver/TSolver.ts';
import { InvalidStateError } from '../solver/errors/InvalidStateError.ts';
import { standardSolverFactory } from '../solver/standardSolverFactory.ts';

import { Enumeration, EnumerationValue } from 'scenerystack/phet-core';

type CanSolve = (board: TBoard, state: TState<TCompleteData>) => boolean;

// EASY and MEDIUM are checked by the WebAssembly core (wasm/src/cansolve.rs), with the same rules as their TS solvers
const wasmCanSolve = (difficulty: string, groups: RuleGroupLimit[]): CanSolve => {
  return (board, state) => {
    const result = coreCanSolve(board, state, difficulty, () => compilePatternRules(board, groups));
    if (result === null) {
      throw new InvalidStateError('Contradiction while checking difficulty');
    }
    return result;
  };
};

export default class CanSolveDifficulty extends EnumerationValue {
  public readonly canSolve: CanSolve;

  public constructor(
    // The TS solver implementation (the reference the WebAssembly checks are tested against)
    public readonly tsCanSolve: CanSolve,
    wasm?: CanSolve,
  ) {
    super();
    this.canSolve = wasm ?? tsCanSolve;
  }

  public static readonly EASY = new CanSolveDifficulty(
    (board, state) => {
      state = state.clone();

      iterateSolverFactory(
        (board: TBoard, state: TState<TCompleteData>, dirty?: boolean) => {
          const boardPatternBoard = BoardPatternBoard.get(board);

          return new CompositeSolver<TCompleteData, TAnnotatedAction<TCompleteData>>([
            new SafeEdgeToSimpleRegionSolver(board, state),
            new SafeSolvedEdgeSolver(board, state),
            new SafeEdgeToFaceColorSolver(board, state),

            new SimpleVertexSolver(board, state, {
              solveJointToRed: true,
              solveForcedLineToBlack: true,
              solveAlmostEmptyToRed: true,
            }),
            new SimpleFaceSolver(board, state, {
              solveToRed: true,
              solveToBlack: true,
            }),

            new SimpleLoopSolver(board, state, {
              solveToRed: true,
              solveToBlack: true,
              resolveAllRegions: false, // NOTE: this will be faster
            }),

            BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeMixedGroup, 120),
          ]);
        },
        board,
        state,
        true,
      );

      return simpleRegionIsSolved(state);
    },
    wasmCanSolve('EASY', [{ group: generalEdgeMixedGroup, size: 120 }]),
  );

  public static readonly MEDIUM = new CanSolveDifficulty(
    (board, state) => {
      state = state.clone();

      iterateSolverFactory(
        (board: TBoard, state: TState<TCompleteData>, dirty?: boolean) => {
          const boardPatternBoard = BoardPatternBoard.get(board);

          return new CompositeSolver<TCompleteData, TAnnotatedAction<TCompleteData>>([
            new SafeEdgeToSimpleRegionSolver(board, state),
            new SafeSolvedEdgeSolver(board, state),
            new SafeEdgeToFaceColorSolver(board, state),

            new SimpleVertexSolver(board, state, {
              solveJointToRed: true,
              solveForcedLineToBlack: true,
              solveAlmostEmptyToRed: true,
            }),
            new SimpleFaceSolver(board, state, {
              solveToRed: true,
              solveToBlack: true,
            }),

            new SimpleLoopSolver(board, state, {
              solveToRed: true,
              solveToBlack: true,
              resolveAllRegions: false, // NOTE: this will be faster
            }),

            BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeSectorMixedGroup, 50),
            BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeMixedGroup, 360),
          ]);
        },
        board,
        state,
        true,
      );

      return simpleRegionIsSolved(state);
    },
    wasmCanSolve('MEDIUM', [
      { group: generalEdgeSectorMixedGroup, size: 50 },
      { group: generalEdgeMixedGroup, size: 360 },
    ]),
  );

  public static readonly HARD = new CanSolveDifficulty((board, state) => {
    state = state.clone();

    iterateSolverFactory(
      (board: TBoard, state: TState<TCompleteData>, dirty?: boolean) => {
        const boardPatternBoard = BoardPatternBoard.get(board);

        return new CompositeSolver<TCompleteData, TAnnotatedAction<TCompleteData>>([
          new SafeEdgeToSimpleRegionSolver(board, state),
          new SafeSolvedEdgeSolver(board, state),
          new SafeEdgeToFaceColorSolver(board, state),

          new SimpleVertexSolver(board, state, {
            solveJointToRed: true,
            solveForcedLineToBlack: true,
            solveAlmostEmptyToRed: true,
          }),
          new SimpleFaceSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
          }),

          new SimpleLoopSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
            resolveAllRegions: false, // NOTE: this will be faster
          }),

          standardSolverFactory(board, state, dirty),

          new FaceColorDisconnectionSolver(board, state),

          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeColorMixedGroup, 600),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeSectorMixedGroup, 100),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeMixedGroup, 1000),
        ]);
      },
      board,
      state,
      true,
    );

    return simpleRegionIsSolved(state);
  });

  public static readonly VERY_HARD = new CanSolveDifficulty((board, state) => {
    state = state.clone();

    iterateSolverFactory(
      (board: TBoard, state: TState<TCompleteData>, dirty?: boolean) => {
        const boardPatternBoard = BoardPatternBoard.get(board);

        return new CompositeSolver<TCompleteData, TAnnotatedAction<TCompleteData>>([
          new SafeEdgeToSimpleRegionSolver(board, state),
          new SafeSolvedEdgeSolver(board, state),
          new SafeEdgeToFaceColorSolver(board, state),

          new SimpleVertexSolver(board, state, {
            solveJointToRed: true,
            solveForcedLineToBlack: true,
            solveAlmostEmptyToRed: true,
          }),
          new SimpleFaceSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
          }),

          new SimpleLoopSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
            resolveAllRegions: false, // NOTE: this will be faster
          }),

          standardSolverFactory(board, state, dirty),

          new FaceColorDisconnectionSolver(board, state),

          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalAllMixedGroup, 1000),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeColorMixedGroup, 2000),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeSectorMixedGroup, 1000),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeMixedGroup, 3000),
        ]);
      },
      board,
      state,
      true,
    );

    return simpleRegionIsSolved(state);
  });

  public static readonly FULL = new CanSolveDifficulty((board, state) => {
    state = state.clone();

    iterateSolverFactory(
      (board: TBoard, state: TState<TCompleteData>, dirty?: boolean) => {
        const boardPatternBoard = BoardPatternBoard.get(board);

        return new CompositeSolver<TCompleteData, TAnnotatedAction<TCompleteData>>([
          new SafeEdgeToSimpleRegionSolver(board, state),
          new SafeSolvedEdgeSolver(board, state),
          new SafeEdgeToFaceColorSolver(board, state),

          new SimpleVertexSolver(board, state, {
            solveJointToRed: true,
            solveForcedLineToBlack: true,
            solveAlmostEmptyToRed: true,
          }),
          new SimpleFaceSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
          }),

          new SimpleLoopSolver(board, state, {
            solveToRed: true,
            solveToBlack: true,
            resolveAllRegions: false, // NOTE: this will be faster
          }),

          standardSolverFactory(board, state, dirty),

          new FaceColorDisconnectionSolver(board, state),

          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalAllMixedGroup),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeColorMixedGroup),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeSectorMixedGroup),
          BinaryPatternSolver.fromGroup(board, boardPatternBoard, state, generalEdgeMixedGroup),
        ]);
      },
      board,
      state,
      true,
    );

    return simpleRegionIsSolved(state);
  });

  public static readonly NO_LIMIT = new CanSolveDifficulty(() => true);

  public static readonly enumeration = new Enumeration(CanSolveDifficulty);
}
