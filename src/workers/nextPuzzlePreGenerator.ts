import { generateAdditiveConstrainedWithWorker } from './getGenerationAdditiveConstrainedWorker.ts';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';

import { PolygonGenerator } from '../model/board/PolygonGenerator.ts';
import { PolygonGeneratorBoard } from '../model/board/core/PolygonGeneratorBoard.ts';
import { TStructure } from '../model/board/core/TStructure.ts';
import { polygonGenerators } from '../model/board/generators/polygonGenerators.ts';
import { TCompleteData } from '../model/data/combined/TCompleteData.ts';
import { deserializeState } from '../model/data/core/deserializeState.ts';
import FaceValue from '../model/data/face-value/FaceValue.ts';
import DifficultyLevel from '../model/generator/DifficultyLevel.ts';
import { GenerationProgressEvent } from '../model/generator/GenerationProgressEvent.ts';
import { TSolvedPuzzle } from '../model/generator/TSolvedPuzzle.ts';
import { canSolveDifficultyProperty } from '../model/generator/canSolveDifficultyProperty.ts';
import { getSolvedPuzzle } from '../model/generator/getSolvedPuzzle.ts';

import { compressString2, decompressString2 } from '../util/compression.ts';

type PreGeneratedPuzzle = {
  key: string;
  interruptedProperty: TinyProperty<boolean>;
  getPuzzle: () => Promise<TSolvedPuzzle<TStructure, TCompleteData> | null>;
  ready: boolean;
  // Progress so far (since the last reset), so a view opened mid-generation can catch up
  progress: GenerationProgressEvent[];
};

export type { GenerationProgressEvent };

// Persisted to localStorage, so that generation can resume (or a finished puzzle can be used) after a reload
type StoredGenerationState = {
  generator: string;
  parameters: Record<string, any>;
  canSolveDifficulty: string;
  // Compressed JSON of the clean state and solution edges (the board is determined by the settings). Only the minimum
  // is stored, since a full serialized solved puzzle for a large board can approach the localStorage quota.
  puzzle: string | null;
  // Generation with these settings failed, so don't retry on load
  failed?: boolean;
  // Compressed JSON of the unfinished generation's progress since the last reset (see GenerationProgressRecorder), so
  // it can continue where it left off
  progress?: string;
};

// Compact form of a GenerationProgressEvent for storage
type StoredProgressEvent = ['d' | 'm', number, FaceValue];

const STORAGE_KEY = 'nextPuzzleGenerationState';

// At most one puzzle is pre-generated (or being pre-generated) at a time
let preGenerated: PreGeneratedPuzzle | null = null;

// Whether a pre-generated puzzle is ready, or one is being generated
export const preGeneratedPuzzleReadyProperty = new TinyProperty(false);
export const preGeneratingPuzzleProperty = new TinyProperty(false);
// The settings (see getPreGenerationKey) of the puzzle that is ready or being generated
export const preGeneratedKeyProperty = new TinyProperty<string | null>(null);
// Progress of background generation, with the key of the puzzle being generated
export const preGenerationProgressEmitter = new TinyEmitter<[key: string, event: GenerationProgressEvent]>();

// Progress so far for a puzzle being generated in the background with these settings (null if there is none)
export const getPreGenerationProgress = (key: string): GenerationProgressEvent[] | null => {
  return preGenerated?.key === key && !preGenerated.ready ? [...preGenerated.progress] : null;
};

const updateStatus = () => {
  preGeneratedPuzzleReadyProperty.value = !!preGenerated?.ready;
  preGeneratingPuzzleProperty.value = !!preGenerated && !preGenerated.ready;
  preGeneratedKeyProperty.value = preGenerated?.key ?? null;
};

const storeGenerationState = (
  generator: PolygonGenerator,
  parameters: Record<string, any>,
  canSolveDifficulty: DifficultyLevel,
  solvedPuzzle: TSolvedPuzzle<TStructure, TCompleteData> | null,
  failed = false,
  progress: GenerationProgressEvent[] = [],
): void => {
  if (!progress.length) {
    // Replaced by other generation state
    currentRecorder?.stop();
  }
  try {
    const state: StoredGenerationState = {
      generator: generator.name,
      parameters: parameters,
      canSolveDifficulty: canSolveDifficulty.name,
      puzzle:
        solvedPuzzle ?
          compressString2(
            JSON.stringify({
              cleanState: solvedPuzzle.cleanState.serializeState(solvedPuzzle.board),
              blackEdges: [...solvedPuzzle.blackEdges].map((edge) => solvedPuzzle.board.edges.indexOf(edge)),
            }),
          )
        : null,
      failed: failed,
      progress:
        progress.length ?
          compressString2(
            JSON.stringify(
              progress.flatMap((event): StoredProgressEvent[] =>
                event.type === 'reset' ? [] : [[event.type === 'define' ? 'd' : 'm', event.index, event.state]],
              ),
            ),
          )
        : undefined,
    };
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch (e) {
    // e.g. quota exceeded, persistence is best-effort
    console.warn('Failed to persist puzzle generation state', e);
  }
};

const loadProgress = (state: StoredGenerationState): GenerationProgressEvent[] => {
  try {
    const events: StoredProgressEvent[] = state.progress ? JSON.parse(decompressString2(state.progress)!) : [];
    return events.map(([type, index, faceValue]) => ({
      type: type === 'd' ? 'define' : 'minimize',
      index: index,
      state: faceValue,
    }));
  } catch (e) {
    console.warn('Failed to restore puzzle generation progress', e);
    return [];
  }
};

/**
 * Records the progress of a generation (since the last reset) and persists it (throttled), so that if the page is
 * closed, generation can continue where it left off. Only the most recently created recorder persists anything, and
 * storing other generation state (e.g. a finished puzzle) stops it.
 */
export class GenerationProgressRecorder {
  private storeTimeout: ReturnType<typeof setTimeout> | null = null;

  public constructor(
    private readonly generator: PolygonGenerator,
    private readonly parameters: Record<string, any>,
    private readonly canSolveDifficulty: DifficultyLevel,
    public readonly progress: GenerationProgressEvent[] = [],
  ) {
    currentRecorder?.stop();
    currentRecorder = this;
  }

  public record(event: GenerationProgressEvent): void {
    if (event.type === 'reset') {
      this.progress.length = 0;
    }
    this.progress.push(event);

    if (currentRecorder === this && this.storeTimeout === null) {
      this.storeTimeout = setTimeout(() => this.flush(), 500);
    }
  }

  public flush(): void {
    if (this.storeTimeout !== null) {
      clearTimeout(this.storeTimeout);
      this.storeTimeout = null;
    }
    // With no progress, the settings are already stored (and storing without progress would stop this recorder)
    if (currentRecorder === this && this.progress.length) {
      storeGenerationState(this.generator, this.parameters, this.canSolveDifficulty, null, false, this.progress);
    }
  }

  public stop(): void {
    if (this.storeTimeout !== null) {
      clearTimeout(this.storeTimeout);
      this.storeTimeout = null;
    }
    if (currentRecorder === this) {
      currentRecorder = null;
    }
  }
}

let currentRecorder: GenerationProgressRecorder | null = null;

// Don't lose the latest (throttled) progress when the page is closed
if (typeof window !== 'undefined') {
  window.addEventListener('pagehide', () => currentRecorder?.flush());
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') {
      currentRecorder?.flush();
    }
  });
}

const loadGenerationState = (): StoredGenerationState | null => {
  try {
    const string = localStorage.getItem(STORAGE_KEY);
    return string ? JSON.parse(string) : null;
  } catch (e) {
    return null;
  }
};

// Snapshots the settings, since parameters objects can be mutated after this is called
export const getPreGenerationKey = (
  generator: PolygonGenerator,
  parameters: Record<string, any>,
  canSolveDifficulty: DifficultyLevel,
): string => {
  return `${generator.name}/${JSON.stringify(parameters)}/${canSolveDifficulty.name}`;
};

const clearPreGeneratedPuzzle = (): void => {
  if (preGenerated) {
    preGenerated.interruptedProperty.value = true;
    preGenerated = null;
    updateStatus();
  }
};

// Starts generating a puzzle in the background (replacing any other pre-generated puzzle with different settings)
export const startPreGeneratingPuzzle = (
  generator: PolygonGenerator,
  parameters: Record<string, any>,
  canSolveDifficulty: DifficultyLevel,
  // Progress of an interrupted generation with these settings to continue from
  resumeProgress: GenerationProgressEvent[] = [],
): void => {
  const key = getPreGenerationKey(generator, parameters, canSolveDifficulty);

  if (preGenerated?.key === key) {
    return;
  }
  clearPreGeneratedPuzzle();

  parameters = { ...parameters };
  storeGenerationState(generator, parameters, canSolveDifficulty, null);
  const recorder = new GenerationProgressRecorder(generator, parameters, canSolveDifficulty, [...resumeProgress]);
  if (resumeProgress.length) {
    // Keep the progress stored, even if the page is closed before there is more
    recorder.flush();
  }

  const board = PolygonGeneratorBoard.get(generator, parameters);
  const interruptedProperty = new TinyProperty(false);

  const onProgress = (event: GenerationProgressEvent) => {
    recorder.record(event);
    preGenerationProgressEmitter.emit(key, event);
  };
  const faceDefineEmitter = new TinyEmitter<[index: number, state: FaceValue]>();
  const faceMinimizeEmitter = new TinyEmitter<[index: number, state: FaceValue]>();
  const faceResetEmitter = new TinyEmitter();
  faceDefineEmitter.addListener((index, state) => onProgress({ type: 'define', index, state }));
  faceMinimizeEmitter.addListener((index, state) => onProgress({ type: 'minimize', index, state }));
  faceResetEmitter.addListener(() => onProgress({ type: 'reset' }));

  const promise = generateAdditiveConstrainedWithWorker(
    board,
    canSolveDifficulty,
    interruptedProperty,
    faceDefineEmitter,
    faceMinimizeEmitter,
    faceResetEmitter,
    resumeProgress,
  );
  const entry: PreGeneratedPuzzle = {
    key: key,
    interruptedProperty: interruptedProperty,
    getPuzzle: () => promise,
    ready: false,
    progress: recorder.progress,
  };
  preGenerated = entry;
  updateStatus();

  promise.then((result) => {
    recorder.stop();
    if (preGenerated === entry) {
      if (result) {
        entry.ready = true;
        storeGenerationState(generator, parameters, canSolveDifficulty, result);
      } else {
        // Failed, don't retry (including after a reload)
        preGenerated = null;
        storeGenerationState(generator, parameters, canSolveDifficulty, null, true);
      }
      updateStatus();
    }
  });
};

/**
 * If a puzzle was (or is being) pre-generated with these settings, takes it (resolving when it is complete). Otherwise
 * stops any pre-generation with different settings (so it doesn't compete with a new generation) and returns null.
 *
 * Either way, these settings are remembered (without a finished puzzle), so that if the page is closed before the
 * requested puzzle is loaded, generation resumes on the next load.
 */
export const takePreGeneratedPuzzle = (
  generator: PolygonGenerator,
  parameters: Record<string, any>,
  canSolveDifficulty: DifficultyLevel,
): Promise<TSolvedPuzzle<TStructure, TCompleteData> | null> | null => {
  const key = getPreGenerationKey(generator, parameters, canSolveDifficulty);

  storeGenerationState(generator, { ...parameters }, canSolveDifficulty, null);

  if (preGenerated?.key === key) {
    const promise = preGenerated.getPuzzle();
    preGenerated = null;
    updateStatus();
    return promise;
  } else {
    clearPreGeneratedPuzzle();
    return null;
  }
};

// Before anything has been generated, use the settings the Generate panel will show (see GenerateNode)
const getGenerateNodeSettings = (): StoredGenerationState => {
  let storedParameters: Record<string, any> | null = null;
  try {
    storedParameters = JSON.parse(localStorage.getItem('polygonGeneratorParameters') ?? 'null');
  } catch (e) {
    // ignore, use defaults
  }

  const generator =
    polygonGenerators.find((generator) => generator.name === localStorage.getItem('polygonGeneratorProperty')) ??
    polygonGenerators[0];

  const parameters: Record<string, any> = {};
  for (const key of Object.keys(generator.parameters)) {
    parameters[key] =
      storedParameters && key in storedParameters ? storedParameters[key] : generator.defaultParameterValues[key];
  }

  return {
    generator: generator.name,
    parameters: parameters,
    canSolveDifficulty: canSolveDifficultyProperty.value.name,
    puzzle: null,
  };
};

/**
 * Restores the persisted generation state: a finished puzzle is made ready immediately, otherwise generation continues
 * (from its recorded progress) with the last-used settings.
 */
export const resumePreGeneration = (): void => {
  if (preGenerated) {
    return;
  }

  const state = loadGenerationState() ?? getGenerateNodeSettings();

  const generator = polygonGenerators.find((generator) => generator.name === state.generator);
  const canSolveDifficulty = DifficultyLevel.enumeration.getValue(state.canSolveDifficulty);
  if (!generator || !canSolveDifficulty) {
    return;
  }

  if (state.failed) {
    return;
  }

  const storedPuzzle = state.puzzle;
  if (storedPuzzle) {
    // Deserialized only when taken, to keep startup fast
    preGenerated = {
      key: getPreGenerationKey(generator, state.parameters, canSolveDifficulty),
      interruptedProperty: new TinyProperty(false),
      getPuzzle: async () => {
        try {
          const { cleanState, blackEdges } = JSON.parse(decompressString2(storedPuzzle)!);
          const board = PolygonGeneratorBoard.get(generator, state.parameters);
          return getSolvedPuzzle(
            board,
            deserializeState(board, cleanState),
            blackEdges.map((index: number) => board.edges[index]),
          );
        } catch (e) {
          console.warn('Failed to restore pre-generated puzzle', e);
          return null;
        }
      },
      ready: true,
      progress: [],
    };
    updateStatus();
    return;
  }

  startPreGeneratingPuzzle(generator, state.parameters, canSolveDifficulty, loadProgress(state));
};
