/**
 * Loop solver backed by the Rust/WebAssembly core (wasm/, built with `npm run build:wasm`).
 *
 * Boards are registered with the core once (cached per board) and each solve only passes face values and edge states,
 * through linear memory.
 */
import { CompiledRules } from './compilePatternRules.ts';
import { slitherCoreWasmBase64 } from './slitherCoreWasm.ts';

import { TBoard } from '../model/board/core/TBoard.ts';
import { TEdge } from '../model/board/core/TEdge.ts';
import { TState } from '../model/data/core/TState.ts';
import EdgeState from '../model/data/edge-state/EdgeState.ts';
import { TEdgeStateData } from '../model/data/edge-state/TEdgeStateData.ts';
import { TFaceValueData } from '../model/data/face-value/TFaceValueData.ts';

type CoreExports = {
  memory: WebAssembly.Memory;
  alloc: (bytes: number) => number;
  dealloc: (pointer: number, bytes: number) => void;
  board_create: (
    numVertices: number,
    edgeVertices: number,
    numEdges: number,
    faceEdgeOffsets: number,
    numFaces: number,
    faceEdges: number,
    numFaceEdges: number,
  ) => number;
  board_dispose: (handle: number) => void;
  solve: (
    handle: number,
    faceValues: number,
    edgeStates: number,
    maxSolutions: number,
    maxIterations: number,
    conflictBudget: number,
  ) => number;
  output_pointer: () => number;
  output_length: () => number;
  last_conflicts: () => number;
  rules_create: (
    boardHandle: number,
    numSectors: number,
    clauses: number,
    clausesLength: number,
    clauseCount: number,
  ) => number;
  can_solve: (boardHandle: number, rulesHandle: number, faceValues: number) => number;
  topology_create: (
    numVertices: number,
    numEdges: number,
    numFaces: number,
    halfEdges: number,
    numHalfEdges: number,
    vertexOffsets: number,
    vertexIncoming: number,
    faceOffsets: number,
    faceHalfEdges: number,
  ) => number;
  can_solve_hard: (topologyHandle: number, rulesHandle: number, faceValues: number) => number;
  gen_create: (
    boardHandle: number,
    topologyHandle: number,
    rulesHandle: number,
    easierRulesHandle: number,
    soundRulesHandle: number,
    check: number,
    seedLow: number,
    seedHigh: number,
    resume: number,
    resumeLength: number,
  ) => number;
  gen_step: (
    handle: number,
    boardHandle: number,
    topologyHandle: number,
    rulesHandle: number,
    easierRulesHandle: number,
    soundRulesHandle: number,
    steps: number,
  ) => number;
  gen_result: (handle: number) => void;
  gen_dispose: (handle: number) => void;
};

let core: CoreExports | null = null;

const decodeBase64 = (base64: string): Uint8Array => {
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
};

const getCore = (): CoreExports => {
  if (!core) {
    // Synchronous, since solving is synchronous (the module is small enough for main-thread compilation)
    const module = new WebAssembly.Module(decodeBase64(slitherCoreWasmBase64) as BufferSource);
    core = new WebAssembly.Instance(module, {}).exports as unknown as CoreExports;
  }
  return core;
};

type CoreBoard = {
  handle: number;
  // Reused per-solve input buffers (in core memory)
  faceValuesPointer: number;
  edgeStatesPointer: number;
};

const boards = new WeakMap<TBoard, CoreBoard>();

// Releases a board's core memory once the board is garbage collected (FinalizationRegistry isn't in our TS lib target)
type HeldBoard = CoreBoard & { numFaces: number; numEdges: number };
const FinalizationRegistryConstructor = (
  globalThis as unknown as {
    FinalizationRegistry?: new (cleanup: (held: HeldBoard) => void) => {
      register: (target: object, held: HeldBoard) => void;
    };
  }
).FinalizationRegistry;
const registry =
  FinalizationRegistryConstructor ?
    new FinalizationRegistryConstructor((held) => {
      const core = getCore();
      core.board_dispose(held.handle);
      core.dealloc(held.faceValuesPointer, held.numFaces);
      core.dealloc(held.edgeStatesPointer, held.numEdges);
    })
  : null;

const withU32Array = <T>(values: ArrayLike<number>, callback: (pointer: number) => T): T => {
  const core = getCore();
  const bytes = values.length * 4;
  const pointer = core.alloc(bytes);
  new Uint32Array(core.memory.buffer, pointer, values.length).set(values);
  try {
    return callback(pointer);
  } finally {
    core.dealloc(pointer, bytes);
  }
};

const getCoreBoard = (board: TBoard): CoreBoard => {
  let coreBoard = boards.get(board);
  if (coreBoard) {
    return coreBoard;
  }

  const core = getCore();
  const vertexIndices = new Map(board.vertices.map((vertex, index) => [vertex, index]));
  const edgeIndices = new Map(board.edges.map((edge, index) => [edge, index]));

  const edgeVertices = new Uint32Array(board.edges.length * 2);
  board.edges.forEach((edge, index) => {
    edgeVertices[2 * index] = vertexIndices.get(edge.start)!;
    edgeVertices[2 * index + 1] = vertexIndices.get(edge.end)!;
  });

  const faceEdgeOffsets = new Uint32Array(board.faces.length + 1);
  const faceEdges: number[] = [];
  board.faces.forEach((face, index) => {
    for (const edge of face.edges) {
      faceEdges.push(edgeIndices.get(edge)!);
    }
    faceEdgeOffsets[index + 1] = faceEdges.length;
  });

  const handle = withU32Array(edgeVertices, (edgeVerticesPointer) =>
    withU32Array(faceEdgeOffsets, (faceEdgeOffsetsPointer) =>
      withU32Array(faceEdges, (faceEdgesPointer) =>
        core.board_create(
          board.vertices.length,
          edgeVerticesPointer,
          board.edges.length,
          faceEdgeOffsetsPointer,
          board.faces.length,
          faceEdgesPointer,
          faceEdges.length,
        ),
      ),
    ),
  );

  coreBoard = {
    handle: handle,
    faceValuesPointer: core.alloc(board.faces.length),
    edgeStatesPointer: core.alloc(board.edges.length),
  };
  boards.set(board, coreBoard);
  registry?.register(board, { ...coreBoard, numFaces: board.faces.length, numEdges: board.edges.length });
  return coreBoard;
};

export const BUDGET_EXCEEDED = null;

// Size of the core's linear memory (for profiling)
export const getCoreMemoryBytes = (): number => getCore().memory.buffer.byteLength;

// SAT conflicts in the last solve (for profiling)
export const getLastSolveConflicts = (): number => getCore().last_conflicts();

/**
 * Finds up to maxSolutions solutions (each the edges of the loop, in loop order) for the state's face values and
 * black/red edges, or returns BUDGET_EXCEEDED if it took more than maxIterations SAT models (each excluding the loops
 * found) or conflictBudget conflicts.
 */
export const wasmSolve = (
  board: TBoard,
  state: TState<TEdgeStateData & TFaceValueData>,
  maxSolutions: number,
  maxIterations: number,
  conflictBudget: number,
): TEdge[][] | typeof BUDGET_EXCEEDED => {
  const core = getCore();
  const coreBoard = getCoreBoard(board);

  // (Views are created after any allocation, since memory growth detaches old buffers)
  const faceValues = new Int8Array(core.memory.buffer, coreBoard.faceValuesPointer, board.faces.length);
  board.faces.forEach((face, index) => {
    const value = state.getFaceValue(face);
    faceValues[index] = value === null ? -1 : value;
  });
  const edgeStates = new Uint8Array(core.memory.buffer, coreBoard.edgeStatesPointer, board.edges.length);
  board.edges.forEach((edge, index) => {
    const edgeState = state.getEdgeState(edge);
    edgeStates[index] =
      edgeState === EdgeState.BLACK ? 1
      : edgeState === EdgeState.RED ? 2
      : 0;
  });

  const count = core.solve(
    coreBoard.handle,
    coreBoard.faceValuesPointer,
    coreBoard.edgeStatesPointer,
    maxSolutions,
    maxIterations,
    conflictBudget,
  );
  if (count < 0) {
    return BUDGET_EXCEEDED;
  }

  const output = new Uint32Array(core.memory.buffer, core.output_pointer(), core.output_length());
  const solutions: TEdge[][] = [];
  let offset = 1;
  for (let i = 0; i < count; i++) {
    const length = output[offset++];
    const solution: TEdge[] = new Array(length);
    for (let j = 0; j < length; j++) {
      solution[j] = board.edges[output[offset++]];
    }
    solutions.push(solution);
  }
  return solutions;
};

const rulesHandles = new WeakMap<TBoard, Map<string, number>>();

// Compiled rules registered with the core, once per board and key
const getRulesHandle = (board: TBoard, rulesKey: string, getRules: () => CompiledRules): number => {
  const core = getCore();
  let boardRules = rulesHandles.get(board);
  if (!boardRules) {
    boardRules = new Map();
    rulesHandles.set(board, boardRules);
  }
  let rulesHandle = boardRules.get(rulesKey);
  if (rulesHandle === undefined) {
    const rules = getRules();
    rulesHandle = withU32Array(rules.clauses, (pointer) =>
      core.rules_create(getCoreBoard(board).handle, rules.numSectors, pointer, rules.clauses.length, rules.clauseCount),
    );
    boardRules.set(rulesKey, rulesHandle);
  }
  return rulesHandle;
};
const topologyHandles = new WeakMap<TBoard, number>();

// The board's half-edge topology, in the TS board's orders (see wasm/src/topology.rs)
const getTopologyHandle = (board: TBoard): number => {
  let handle = topologyHandles.get(board);
  if (handle === undefined) {
    const core = getCore();
    const vertexIndices = new Map(board.vertices.map((vertex, index) => [vertex, index]));
    const edgeIndices = new Map(board.edges.map((edge, index) => [edge, index]));
    const faceIndices = new Map(board.faces.map((face, index) => [face, index]));
    const halfEdgeIndices = new Map(board.halfEdges.map((halfEdge, index) => [halfEdge, index]));

    const halfEdges = new Uint32Array(board.halfEdges.length * 6);
    board.halfEdges.forEach((halfEdge, index) => {
      halfEdges.set(
        [
          vertexIndices.get(halfEdge.start)!,
          vertexIndices.get(halfEdge.end)!,
          edgeIndices.get(halfEdge.edge)!,
          halfEdge.face ? faceIndices.get(halfEdge.face)! : 0xffffffff,
          halfEdgeIndices.get(halfEdge.next)!,
          halfEdgeIndices.get(halfEdge.reversed)!,
        ],
        index * 6,
      );
    });
    const toCsr = <T>(items: T[], getList: (item: T) => unknown[]): [number[], number[]] => {
      const offsets = [0];
      const list: number[] = [];
      for (const item of items) {
        for (const halfEdge of getList(item)) {
          list.push(halfEdgeIndices.get(halfEdge as TBoard['halfEdges'][number])!);
        }
        offsets.push(list.length);
      }
      return [offsets, list];
    };
    const [vertexOffsets, vertexIncoming] = toCsr(board.vertices, (vertex) => vertex.incomingHalfEdges);
    const [faceOffsets, faceHalfEdges] = toCsr(board.faces, (face) => face.halfEdges);

    handle = withU32Array(halfEdges, (halfEdgesPointer) =>
      withU32Array(vertexOffsets, (vertexOffsetsPointer) =>
        withU32Array(vertexIncoming, (vertexIncomingPointer) =>
          withU32Array(faceOffsets, (faceOffsetsPointer) =>
            withU32Array(faceHalfEdges, (faceHalfEdgesPointer) =>
              core.topology_create(
                board.vertices.length,
                board.edges.length,
                board.faces.length,
                halfEdgesPointer,
                board.halfEdges.length,
                vertexOffsetsPointer,
                vertexIncomingPointer,
                faceOffsetsPointer,
                faceHalfEdgesPointer,
              ),
            ),
          ),
        ),
      ),
    );
    topologyHandles.set(board, handle);
  }
  return handle;
};

/**
 * Whether the deduction rules solve the puzzle from its face values (see wasm/src/cansolve.rs). The rules are compiled
 * (by getRules, e.g. with compilePatternRules) and registered once per board and key. Returns null on a contradiction.
 */
export const wasmCanSolve = (
  board: TBoard,
  state: TFaceValueData,
  rulesKey: string,
  getRules: () => CompiledRules,
  // The HARD rule set (wasm/src/engine.rs), instead of EASY/MEDIUM's (wasm/src/cansolve.rs)
  hard = false,
): boolean | null => {
  const core = getCore();
  const coreBoard = getCoreBoard(board);

  const rulesHandle = getRulesHandle(board, rulesKey, getRules);

  const faceValues = new Uint8Array(core.memory.buffer, coreBoard.faceValuesPointer, board.faces.length);
  board.faces.forEach((face, index) => {
    const value = state.getFaceValue(face);
    faceValues[index] = value === null ? 0xff : value;
  });

  const result =
    hard ?
      core.can_solve_hard(getTopologyHandle(board), rulesHandle, coreBoard.faceValuesPointer)
    : core.can_solve(coreBoard.handle, rulesHandle, coreBoard.faceValuesPointer);
  return result < 0 ? null : result === 1;
};

/** The difficulty check a generation uses (see CanSolveDifficulty) */
export type WasmDifficultyRules = {
  key: string;
  getRules: () => CompiledRules;
  hard: boolean;
  // An easier difficulty's EASY/MEDIUM-style rules (a subset of these), tried first since they're cheaper
  easier?: { key: string; getRules: () => CompiledRules };
  // Rules that don't assume uniqueness (no highlander rules), so a puzzle they solve is unique without a SAT check.
  // `impliesDifficulty`: they're a subset of the difficulty's rules. `only`: minimization only removes values while they
  // still solve the puzzle (otherwise it falls back to the difficulty check and SAT).
  sound?: { key: string; getRules: () => CompiledRules; impliesDifficulty: boolean; only: boolean };
};

export type WasmGenerationEvent =
  { type: 'define' | 'minimize'; index: number; state: number | null } | { type: 'reset' };

const NONE = 0xffffffff;

/**
 * Generates a puzzle (wasm/src/generator.rs), in steps: progress events are reported as they happen, and it yields to
 * the event loop between steps (throwing from yieldAndCheck when interrupted). Resumes from earlier progress events if
 * given. Returns the puzzle's face values (null for none) and its solution's edges, or null if generation failed.
 */
export const wasmGenerate = async (
  board: TBoard,
  rules: WasmDifficultyRules | null,
  seed: [number, number],
  resume: WasmGenerationEvent[],
  onEvent: (event: WasmGenerationEvent) => void,
  yieldAndCheck: () => Promise<void>,
): Promise<{ faceValues: (number | null)[]; solutionEdges: TEdge[] } | null> => {
  const core = getCore();
  const boardHandle = getCoreBoard(board).handle;
  // With sound-only rules that imply the difficulty, they're the whole check (see generator.rs), so the (large) main and
  // easier rules aren't compiled
  const soundOnly = !!rules?.sound && rules.sound.only && rules.sound.impliesDifficulty;
  const rulesHandle = rules && !soundOnly ? getRulesHandle(board, rules.key, rules.getRules) : NONE;
  const easierRulesHandle =
    rules?.easier && !soundOnly ? getRulesHandle(board, rules.easier.key, rules.easier.getRules) : NONE;
  const soundRulesHandle =
    rules?.sound ?
      getRulesHandle(board, rules.sound.key, rules.sound.getRules) |
      (rules.sound.impliesDifficulty ? 0x80000000 : 0) |
      (rules.sound.only ? 0x40000000 : 0)
    : NONE;
  const topologyHandle = rules?.hard ? getTopologyHandle(board) : NONE;
  const check =
    !rules ? 0
    : rules.hard ? 2
    : 1;

  const resumeData: number[] = [];
  for (const event of resume) {
    if (event.type === 'reset') {
      resumeData.push(2, 0, 0);
    } else {
      resumeData.push(event.type === 'define' ? 0 : 1, event.index, event.state === null ? 0xff : event.state);
    }
  }
  const handle = withU32Array(resumeData, (pointer) =>
    core.gen_create(
      boardHandle,
      topologyHandle,
      rulesHandle,
      easierRulesHandle,
      soundRulesHandle >>> 0,
      check,
      seed[0],
      seed[1],
      pointer,
      resumeData.length,
    ),
  );

  try {
    let lastYield = performance.now();
    while (true) {
      const status = core.gen_step(
        handle,
        boardHandle,
        topologyHandle,
        rulesHandle,
        easierRulesHandle,
        soundRulesHandle >>> 0,
        1,
      );
      const output = new Uint32Array(core.memory.buffer, core.output_pointer(), core.output_length()).slice();
      for (let i = 0; i < output.length; i += 3) {
        const type = output[i];
        onEvent(
          type === 2 ?
            { type: 'reset' }
          : {
              type: type === 0 ? 'define' : 'minimize',
              index: output[i + 1],
              state: output[i + 2] === 0xff ? null : output[i + 2],
            },
        );
      }
      if (status === 2) {
        return null;
      }
      if (status === 1) {
        core.gen_result(handle);
        const result = new Uint32Array(core.memory.buffer, core.output_pointer(), core.output_length());
        const faceValues = Array.from(result.subarray(0, board.faces.length), (value) =>
          value === 0xff ? null : value,
        );
        const solutionEdges = Array.from(result.subarray(board.faces.length), (index) => board.edges[index]);
        return { faceValues, solutionEdges };
      }
      // Yield regularly (steps take up to tens of milliseconds)
      if (performance.now() - lastYield > 8) {
        await yieldAndCheck();
        lastYield = performance.now();
      }
    }
  } finally {
    core.gen_dispose(handle);
  }
};
