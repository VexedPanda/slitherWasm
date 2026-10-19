/**
 * Where canSolve's time goes during generation, per solver class: node test/node/run.mjs test/node/canSolveProfile.ts
 * <size> <difficulty>
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import { CompleteData } from '../../src/model/data/combined/CompleteData.ts';
import CanSolveDifficulty from '../../src/model/generator/CanSolveDifficulty.ts';
import { generateAdditiveConstrained } from '../../src/model/generator/generateAdditiveConstrained.ts';
import { BinaryPatternSolver } from '../../src/model/solver/BinaryPatternSolver.ts';
import { FaceColorDisconnectionSolver } from '../../src/model/solver/FaceColorDisconnectionSolver.ts';
import { SafeEdgeToFaceColorSolver } from '../../src/model/solver/SafeEdgeToFaceColorSolver.ts';
import { SafeEdgeToSimpleRegionSolver } from '../../src/model/solver/SafeEdgeToSimpleRegionSolver.ts';
import { SafeSolvedEdgeSolver } from '../../src/model/solver/SafeSolvedEdgeSolver.ts';
import { SimpleFaceSolver } from '../../src/model/solver/SimpleFaceSolver.ts';
import { SimpleLoopSolver } from '../../src/model/solver/SimpleLoopSolver.ts';
import { SimpleVertexSolver } from '../../src/model/solver/SimpleVertexSolver.ts';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';

const size = Number(process.argv[2] ?? 20);
const difficulty = CanSolveDifficulty.enumeration.getValue(process.argv[3] ?? 'MEDIUM');

const totals = new Map<string, { ms: number; calls: number; actions: number }>();
const record = (name: string, ms: number, action: boolean) => {
  const total = totals.get(name) ?? { ms: 0, calls: 0, actions: 0 };
  total.ms += ms;
  total.calls++;
  total.actions += action ? 1 : 0;
  totals.set(name, total);
};
const wrap = (name: string, prototype: { nextAction: () => unknown }) => {
  const original = prototype.nextAction;
  prototype.nextAction = function (this: unknown) {
    const start = performance.now();
    const action = original.call(this);
    record(name, performance.now() - start, !!action);
    return action;
  };
};
const constructorTime = (name: string, cls: { prototype: object }) => {
  void name;
  void cls;
};
for (const [name, cls] of Object.entries({
  BinaryPatternSolver,
  FaceColorDisconnectionSolver,
  SafeEdgeToFaceColorSolver,
  SafeEdgeToSimpleRegionSolver,
  SafeSolvedEdgeSolver,
  SimpleFaceSolver,
  SimpleLoopSolver,
  SimpleVertexSolver,
})) {
  wrap(name, cls.prototype as unknown as { nextAction: () => unknown });
  constructorTime(name, cls);
}
const cloneOriginal = CompleteData.prototype.clone;
CompleteData.prototype.clone = function (this: CompleteData) {
  const start = performance.now();
  const result = cloneOriginal.call(this);
  record('CompleteData.clone', performance.now() - start, false);
  return result;
};

let canSolveMs = 0;
let canSolveCalls = 0;
// Profile the TS solvers (the reference implementation)
const original = difficulty.tsCanSolve;
(difficulty as unknown as { canSolve: typeof original }).canSolve = (board, state) => {
  const start = performance.now();
  try {
    return original(board, state);
  } finally {
    canSolveMs += performance.now() - start;
    canSolveCalls++;
  }
};

const board = PolygonGeneratorBoard.get(
  polygonGenerators.find((g) => g.name === 'Square')!,
  { width: size, height: size },
);
const start = performance.now();
await generateAdditiveConstrained(
  board,
  difficulty,
  new TinyProperty(false),
  new TinyEmitter(),
  new TinyEmitter(),
  new TinyEmitter(),
);
const total = performance.now() - start;

console.log(
  `generation ${(total / 1000).toFixed(1)} s, canSolve ${(canSolveMs / 1000).toFixed(1)} s over ${canSolveCalls} calls (${(canSolveMs / canSolveCalls).toFixed(1)} ms each)`,
);
const accounted = [...totals.values()].reduce((sum, t) => sum + t.ms, 0);
for (const [name, t] of [...totals.entries()].sort((a, b) => b[1].ms - a[1].ms)) {
  console.log(`  ${name.padEnd(30)} ${(t.ms / 1000).toFixed(2)} s  ${t.calls} calls  ${t.actions} actions`);
}
console.log(
  `  (other canSolve overhead: ${((canSolveMs - accounted) / 1000).toFixed(2)} s, e.g. solver construction and applying actions)`,
);
process.exit(0);
