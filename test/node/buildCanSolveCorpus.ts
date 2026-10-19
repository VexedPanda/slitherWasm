/**
 * Records the difficulty checks (CanSolveDifficulty.canSolve) that real puzzle generations make, with their results, as a
 * fixture for checking other implementations: node test/node/run.mjs test/node/buildCanSolveCorpus.ts
 *
 * Each case is a board, a difficulty, and the face values (the states generation checks only have face values).
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import CanSolveDifficulty from '../../src/model/generator/CanSolveDifficulty.ts';
import { generateAdditiveConstrained } from '../../src/model/generator/generateAdditiveConstrained.ts';
import { writeFileSync } from 'node:fs';
import { gzipSync } from 'node:zlib';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';
import { dotRandom } from 'scenerystack/dot';

type Spec = { generator: string; parameters: Record<string, unknown>; difficulty: string; seed: number };

const specs: Spec[] = [];
for (const difficulty of ['EASY', 'MEDIUM', 'HARD']) {
  for (const seed of [1, 2]) {
    specs.push({ generator: 'Square', parameters: { width: 10, height: 10 }, difficulty, seed });
  }
  specs.push({ generator: 'Square', parameters: { width: 15, height: 15 }, difficulty, seed: 3 });
  specs.push({
    generator: 'Hexagonal',
    parameters: { radius: 4, isPointyTop: true, holeRadius: 0 },
    difficulty,
    seed: 4,
  });
}
specs.push({ generator: 'Square', parameters: { width: 20, height: 20 }, difficulty: 'MEDIUM', seed: 5 });

export type CanSolveCase = { board: number; difficulty: string; faces: string; result: boolean };
const boards: { generator: string; parameters: Record<string, unknown> }[] = [];
const cases: CanSolveCase[] = [];

for (const spec of specs) {
  const generator = polygonGenerators.find((g) => g.name === spec.generator)!;
  const parameters = { ...generator.defaultParameterValues, ...spec.parameters };
  const board = PolygonGeneratorBoard.get(generator, parameters);
  let boardIndex = boards.findIndex(
    (b) => b.generator === spec.generator && JSON.stringify(b.parameters) === JSON.stringify(parameters),
  );
  if (boardIndex < 0) {
    boards.push({ generator: spec.generator, parameters });
    boardIndex = boards.length - 1;
  }

  const difficulty = CanSolveDifficulty.enumeration.getValue(spec.difficulty);
  // Record the TS solvers' results (the reference)
  const original = difficulty.canSolve;
  const recorded = { canSolve: original };
  (difficulty as unknown as typeof recorded).canSolve = (b, state) => {
    const result = difficulty.tsCanSolve(b, state);
    cases.push({
      board: boardIndex,
      difficulty: spec.difficulty,
      faces: b.faces.map((face) => state.getFaceValue(face) ?? '.').join(''),
      result,
    });
    return result;
  };

  dotRandom.setSeed(spec.seed);
  const start = performance.now();
  await generateAdditiveConstrained(
    board,
    difficulty,
    new TinyProperty(false),
    new TinyEmitter(),
    new TinyEmitter(),
    new TinyEmitter(),
  );
  (difficulty as unknown as typeof recorded).canSolve = original;
  console.log(
    `${spec.generator} ${JSON.stringify(spec.parameters)} ${spec.difficulty}: ${((performance.now() - start) / 1000).toFixed(1)} s, ${cases.length} cases so far`,
  );
}

// Deduplicate
const seen = new Set<string>();
const unique = cases.filter((c) => {
  const key = `${c.board}|${c.difficulty}|${c.faces}`;
  return seen.has(key) ? false : (seen.add(key), true);
});
writeFileSync('test/node/fixtures/canSolveCorpus.json.gz', gzipSync(JSON.stringify({ boards, cases: unique })));
console.log(`${unique.length} cases (${unique.filter((c) => c.result).length} solvable)`);
process.exit(0);
