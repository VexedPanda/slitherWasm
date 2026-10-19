/**
 * Checks that the WebAssembly solver finds the same number of solutions as the MiniSat-based satSolve, on random states
 * of several board types: node test/node/run.mjs test/node/solverEquivalence.ts [trials]
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { TBoard } from '../../src/model/board/core/TBoard.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import { CompleteData } from '../../src/model/data/combined/CompleteData.ts';
import EdgeState from '../../src/model/data/edge-state/EdgeState.ts';
import { generateFaceAdditive } from '../../src/model/generator/generateFaceAdditive.ts';
import { satSolveMiniSat as satSolve } from '../../src/model/solver/MiniSatSolver.ts';
import { MaximumSolverIterationsError } from '../../src/model/solver/errors/MaximumSolverIterationsError.ts';
import { wasmSolve } from '../../src/wasm/slitherCore.ts';

import { dotRandom } from 'scenerystack/dot';

const trials = Number(process.argv[2] ?? 200);

const boards: { name: string; board: TBoard }[] = [];
for (const [name, parameters] of [
  ['Square', { width: 4, height: 4 }],
  ['Square', { width: 6, height: 5 }],
  ['Hexagonal', { radius: 3, isPointyTop: true, holeRadius: 0 }],
] as const) {
  const generator = polygonGenerators.find((g) => g.name === name)!;
  const fullParameters = { ...generator.defaultParameterValues, ...parameters };
  boards.push({
    name: `${name} ${JSON.stringify(parameters)}`,
    board: PolygonGeneratorBoard.get(generator, fullParameters),
  });
}
for (const generator of polygonGenerators.slice(1, 6)) {
  if (generator.name !== 'Hexagonal') {
    const parameters = { ...generator.defaultParameterValues, width: 4, height: 4 };
    boards.push({ name: generator.name, board: PolygonGeneratorBoard.get(generator, parameters) });
  }
}

const countSat = (board: TBoard, state: CompleteData): number | null => {
  try {
    return Math.min(satSolve(board, state, { maxIterations: 10000, failOnMultipleSolutions: false }).length, 3);
  } catch (e) {
    if (e instanceof MaximumSolverIterationsError) {
      return null;
    }
    throw e;
  }
};

let compared = 0;
let mismatches = 0;
let lenient = 0;
const counts = [0, 0, 0, 0];

for (let trial = 0; trial < trials; trial++) {
  const { name, board } = boards[trial % boards.length];

  // Start from a real puzzle, then perturb it: drop clues, change some, and mark some edges
  const puzzle = await generateFaceAdditive(board);
  const state = CompleteData.fromFaces(board, (face) => {
    const value = puzzle.cleanState.getFaceValue(face);
    const r = dotRandom.nextDouble();
    if (r < 0.35) {
      return null;
    } else if (r < 0.42) {
      return dotRandom.nextIntBetween(0, face.edges.length - 1);
    }
    return value;
  });
  const markRate = dotRandom.nextDouble() * 0.3;
  for (const edge of board.edges) {
    if (dotRandom.nextDouble() < markRate) {
      const correct = puzzle.blackEdges.has(edge) ? EdgeState.BLACK : EdgeState.RED;
      const wrong = correct === EdgeState.BLACK ? EdgeState.RED : EdgeState.BLACK;
      state.setEdgeState(edge, dotRandom.nextDouble() < 0.9 ? correct : wrong);
    }
  }
  if (
    !board.faces.some((face) => state.getFaceValue(face) !== null) ||
    !board.edges.some((edge) => state.getEdgeState(edge) === EdgeState.WHITE)
  ) {
    continue;
  }

  const expected = countSat(board, state);
  if (expected === null) {
    continue;
  }
  const solutions = wasmSolve(board, state, 3, 1e9);
  const actual = solutions === null ? -1 : solutions.length;

  compared++;
  counts[Math.min(expected, 3)]++;
  if (actual !== expected) {
    // satSolve also counts loops that leave given black edges outside the loop, or that overfill a face with given
    // black edges, which aren't solutions. Only count those that are.
    const satSolutions = satSolve(board, state, { maxIterations: 10000, failOnMultipleSolutions: false });
    const valid = satSolutions.filter((solution) => {
      const loop = new Set(solution);
      return (
        board.edges.every((edge) => state.getEdgeState(edge) !== EdgeState.BLACK || loop.has(edge)) &&
        board.edges.every((edge) => state.getEdgeState(edge) !== EdgeState.RED || !loop.has(edge)) &&
        board.faces.every((face) => {
          const value = state.getFaceValue(face);
          return value === null || face.edges.filter((edge) => loop.has(edge)).length === value;
        })
      );
    });
    if (Math.min(valid.length, 3) === actual) {
      lenient++;
      continue;
    }
    mismatches++;
    console.log(`MISMATCH on ${name}: satSolve ${expected} (${valid.length} valid), wasm ${actual}`);
  }
}

console.log(
  `compared ${compared} states (0/1/2/3+ solutions: ${counts.join('/')}), ${lenient} only differing by invalid satSolve loops, ${mismatches} mismatches`,
);
process.exit(mismatches ? 1 : 0);
