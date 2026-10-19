import { RuleGroupLimit, compilePatternRules } from '../../wasm/compilePatternRules.ts';
import { WasmDifficultyRules, wasmCanSolve as coreCanSolve } from '../../wasm/slitherCore.ts';
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

// EASY, MEDIUM and HARD are checked by the WebAssembly core (wasm/src/cansolve.rs, and wasm/src/engine.rs for HARD's
// larger rule set), with the same rules as their TS solvers
type WasmCheck = { canSolve: CanSolve; getRules: (board: TBoard) => WasmDifficultyRules };

// The rule groups of EASY and MEDIUM (each difficulty's rules include the easier ones', see generator.rs)
const EASY_GROUPS: RuleGroupLimit[] = [{ group: generalEdgeMixedGroup, size: 120 }];
const MEDIUM_GROUPS: RuleGroupLimit[] = [
  { group: generalEdgeSectorMixedGroup, size: 50 },
  { group: generalEdgeMixedGroup, size: 360 },
];
// Without the rules that assume uniqueness (subsets of the difficulties' rules), see WasmDifficultyRules.sound
const withoutHighlander = (groups: RuleGroupLimit[]): RuleGroupLimit[] =>
  groups.map((limit) => ({ ...limit, withoutHighlander: true }));

const wasmCheck = (
  difficulty: string,
  groups: RuleGroupLimit[],
  hard = false,
  easier?: { key: string; groups: RuleGroupLimit[] },
  sound?: { key: string; groups: RuleGroupLimit[]; only: boolean },
): WasmCheck => {
  const getRules = (board: TBoard): WasmDifficultyRules => ({
    key: difficulty,
    getRules: () => compilePatternRules(board, groups),
    hard: hard,
    easier: easier && { key: easier.key, getRules: () => compilePatternRules(board, easier.groups) },
    sound: sound && {
      key: sound.key,
      getRules: () => compilePatternRules(board, sound.groups),
      impliesDifficulty: true,
      only: sound.only,
    },
  });
  return {
    canSolve: (board, state) => {
      const rules = getRules(board);
      const result = coreCanSolve(board, state, rules.key, rules.getRules, hard);
      if (result === null) {
        throw new InvalidStateError('Contradiction while checking difficulty');
      }
      return result;
    },
    getRules: getRules,
  };
};

export default class CanSolveDifficulty extends EnumerationValue {
  public readonly canSolve: CanSolve;

  // The WebAssembly core's rules for this difficulty (used by puzzle generation), if it has them
  public readonly getWasmRules: ((board: TBoard) => WasmDifficultyRules) | null;

  public constructor(
    // The TS solver implementation (the reference the WebAssembly checks are tested against)
    public readonly tsCanSolve: CanSolve,
    wasm?: WasmCheck,
  ) {
    super();
    this.canSolve = wasm?.canSolve ?? tsCanSolve;
    this.getWasmRules = wasm?.getRules ?? null;
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
    // Minimizing only while the rules without highlander rules solve the puzzle avoids SAT uniqueness checks (the
    // bottleneck on large boards)
    wasmCheck('EASY', EASY_GROUPS, false, undefined, {
      key: 'EASY-sound',
      groups: withoutHighlander(EASY_GROUPS),
      only: true,
    }),
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
    wasmCheck(
      'MEDIUM',
      MEDIUM_GROUPS,
      false,
      { key: 'EASY', groups: EASY_GROUPS },
      { key: 'MEDIUM-sound', groups: withoutHighlander(MEDIUM_GROUPS), only: true },
    ),
  );

  public static readonly HARD = new CanSolveDifficulty(
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
    },
    wasmCheck(
      'HARD',
      [
        { group: generalEdgeColorMixedGroup, size: 600 },
        { group: generalEdgeSectorMixedGroup, size: 100 },
        { group: generalEdgeMixedGroup, size: 1000 },
      ],
      true,
      { key: 'MEDIUM', groups: MEDIUM_GROUPS },
      { key: 'MEDIUM-sound', groups: withoutHighlander(MEDIUM_GROUPS), only: false },
    ),
  );

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
