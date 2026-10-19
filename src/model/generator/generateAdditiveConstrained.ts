import { wasmGenerate } from '../../wasm/slitherCore.ts';
import { TBoard } from '../board/core/TBoard.ts';
import { TFace } from '../board/core/TFace.ts';
import { TStructure } from '../board/core/TStructure.ts';
import { CompleteData } from '../data/combined/CompleteData.ts';
import { TCompleteData } from '../data/combined/TCompleteData.ts';
import FaceValue from '../data/face-value/FaceValue.ts';
import { incrementalSatSolve } from '../solver/IncrementalSATSolver.ts';
import { InterruptedError } from '../solver/errors/InterruptedError.ts';
import CanSolveDifficulty from './CanSolveDifficulty.ts';
import { GenerationProgressEvent } from './GenerationProgressEvent.ts';
import { TSolvedPuzzle } from './TSolvedPuzzle.ts';
import { generateFaceAdditive } from './generateFaceAdditive.ts';
import { getSolvedPuzzle } from './getSolvedPuzzle.ts';
import { greedyFaceMinimize } from './greedyFaceMinimize.ts';
import { withAllFacesFilled } from './withAllFacesFilled.ts';

import { TEmitter, TReadOnlyProperty } from 'scenerystack/axon';
import { dotRandom } from 'scenerystack/dot';

import { interruptableSleep } from '../../util/interruptableSleep.ts';

/**
 * Generates a puzzle: values are added until it has a unique solution, then filled in (unless there's no difficulty
 * limit), then removed while it stays unique and solvable at the difficulty. Runs in the WebAssembly core
 * (wasm/src/generator.rs) when the difficulty check is there, otherwise in TS (generateAdditiveConstrainedTS).
 */
export const generateAdditiveConstrained = async (
  board: TBoard,
  canSolveDifficulty: CanSolveDifficulty,
  interruptedProperty: TReadOnlyProperty<boolean>,
  faceDefineEmitter: TEmitter<[index: number, state: FaceValue]>,
  faceMinimizeEmitter: TEmitter<[index: number, state: FaceValue]>,
  faceResetEmitter: TEmitter,
  // Progress (since the last reset) of an earlier, interrupted generation to continue from. These events are not
  // emitted again.
  resumeProgress: GenerationProgressEvent[] = [],
): Promise<TSolvedPuzzle<TStructure, TCompleteData> | null> => {
  const rules = canSolveDifficulty.getWasmRules?.(board) ?? null;
  if (!rules && canSolveDifficulty !== CanSolveDifficulty.NO_LIMIT) {
    return generateAdditiveConstrainedTS(
      board,
      canSolveDifficulty,
      interruptedProperty,
      faceDefineEmitter,
      faceMinimizeEmitter,
      faceResetEmitter,
      resumeProgress,
    );
  }

  try {
    // Seeded from dotRandom, so seeding it makes generation deterministic
    const seed: [number, number] = [
      Math.floor(dotRandom.nextDouble() * 2 ** 32),
      Math.floor(dotRandom.nextDouble() * 2 ** 32),
    ];
    const result = await wasmGenerate(
      board,
      rules,
      seed,
      resumeProgress,
      (event) => {
        if (event.type === 'reset') {
          faceResetEmitter.emit();
        } else if (event.type === 'define') {
          faceDefineEmitter.emit(event.index, event.state);
        } else {
          faceMinimizeEmitter.emit(event.index, event.state);
        }
      },
      () => interruptableSleep(0, interruptedProperty),
    );
    if (!result) {
      return null;
    }
    const cleanState = CompleteData.empty(board);
    board.faces.forEach((face, index) => cleanState.setFaceValue(face, result.faceValues[index]));
    return getSolvedPuzzle(board, cleanState, result.solutionEdges);
  } catch (e) {
    if (e instanceof InterruptedError) {
      return null;
    } else {
      throw e;
    }
  }
};

// The TS implementation (for difficulties without WebAssembly checks)
export const generateAdditiveConstrainedTS = async (
  board: TBoard,
  canSolveDifficulty: CanSolveDifficulty,
  interruptedProperty: TReadOnlyProperty<boolean>,
  faceDefineEmitter: TEmitter<[index: number, state: FaceValue]>,
  faceMinimizeEmitter: TEmitter<[index: number, state: FaceValue]>,
  faceResetEmitter: TEmitter,
  // Progress (since the last reset) of an earlier, interrupted generation to continue from. These events are not
  // emitted again.
  resumeProgress: GenerationProgressEvent[] = [],
): Promise<TSolvedPuzzle<TStructure, TCompleteData> | null> => {
  try {
    const canSolve = canSolveDifficulty.canSolve;

    // Reconstruct the interrupted generation's state (the latest value wins, since a generation attempt can be retried
    // without a reset)
    const definedValues = new Map<TFace, FaceValue>();
    const minimizedValues = new Map<TFace, FaceValue>();
    for (const event of resumeProgress) {
      if (event.type === 'reset') {
        definedValues.clear();
        minimizedValues.clear();
      } else {
        const face = board.faces[event.index];
        if (face) {
          (event.type === 'define' ? definedValues : minimizedValues).set(face, event.state);
        }
      }
    }

    // If it was interrupted while minimizing, continue minimizing (the puzzle being minimized already passed the checks)
    if (minimizedValues.size) {
      try {
        // Set the values (instead of using fromFaces with them) so that the face states stay unconstrained, as in a
        // generated puzzle
        const cleanState = CompleteData.empty(board);
        for (const face of board.faces) {
          cleanState.setFaceValue(
            face,
            minimizedValues.has(face) ? minimizedValues.get(face)! : (definedValues.get(face) ?? null),
          );
        }
        const solutions = incrementalSatSolve(board, cleanState, {
          maxIterations: 10000,
          failOnMultipleSolutions: true,
        });
        if (solutions.length === 1 && canSolve(board, cleanState.clone())) {
          return await greedyFaceMinimize(
            getSolvedPuzzle(board, cleanState, solutions[0]),
            canSolve,
            interruptedProperty,
            faceMinimizeEmitter,
            new Set(minimizedValues.keys()),
          );
        }
      } catch (e) {
        if (e instanceof InterruptedError) {
          throw e;
        }
        console.warn('Failed to resume minimizing', e);
      }
      // Couldn't resume, start over
      faceResetEmitter.emit();
      definedValues.clear();
    }

    let initialFaceValues = definedValues.size ? definedValues : undefined;

    const getUniquePuzzle = async () => {
      const faceValues = initialFaceValues;
      initialFaceValues = undefined;

      if (faceValues) {
        try {
          return await generateFaceAdditive(board, interruptedProperty, faceDefineEmitter, faceValues);
        } catch (e) {
          if (e instanceof InterruptedError) {
            throw e;
          }
          console.warn('Failed to resume generation', e);
          faceResetEmitter.emit();
        }
      }
      return await generateFaceAdditive(board, interruptedProperty, faceDefineEmitter);
    };

    const getMinimizablePuzzle = async () => {
      let uniquePuzzle = await getUniquePuzzle();

      if (canSolveDifficulty === CanSolveDifficulty.NO_LIMIT) {
        return uniquePuzzle;
      } else {
        // TODO: should we do this on everything? probably not, because it ... doesn't change the distribution? BUT might make it harder?

        const blankFaces = board.faces.filter((face) => uniquePuzzle.cleanState.getFaceValue(face) === null);
        const minimizablePuzzle = withAllFacesFilled(uniquePuzzle);
        blankFaces.forEach((face) => {
          faceDefineEmitter.emit(board.faces.indexOf(face), minimizablePuzzle.cleanState.getFaceValue(face));
        });
        return minimizablePuzzle;
      }
    };

    let minimizablePuzzle = await getMinimizablePuzzle();
    while (
      // Don't allow the "fully full" state, e.g. 4 in square, since it will be boring trivial puzzles, and for
      // https://github.com/jonathanolson/slither/issues/2
      minimizablePuzzle.board.faces.some(
        (face) => minimizablePuzzle.solvedState.getFaceValue(face) === face.edges.length,
      ) ||
      !canSolve(minimizablePuzzle.board, minimizablePuzzle.cleanState.clone())
    ) {
      faceResetEmitter.emit();
      minimizablePuzzle = await getMinimizablePuzzle();
    }

    const minimizedPuzzle = await greedyFaceMinimize(
      minimizablePuzzle,
      canSolve,
      interruptedProperty,
      faceMinimizeEmitter,
    );

    // Maybe... let it complete on the screen before we do complicated time consuming things
    interruptableSleep(17, interruptedProperty);

    // console.log(
    //   'total difficulty',
    //   estimateDifficulty(minimizedPuzzle.board, minimizedPuzzle.cleanState, {
    //     solveEdges: true,
    //     solveSectors: false,
    //     solveFaceColors: true,
    //     solveVertexState: false,
    //     solveFaceState: false,
    //     cutoffDifficulty: Number.POSITIVE_INFINITY,
    //     // cutoffDifficulty: 10,
    //   }),
    // );

    return minimizedPuzzle;
  } catch (e) {
    if (e instanceof InterruptedError) {
      // do nothing, we got interrupted and that's fine. Handled elsewhere
      return null;
    } else {
      throw e;
    }
  }
};
