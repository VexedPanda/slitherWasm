/**
 * Checks the WebAssembly difficulty checks against the TS ones, on the difficulty checks recorded from real generations
 * (test/node/fixtures/canSolveCorpus.json.gz, from buildCanSolveCorpus.ts): node test/node/run.mjs
 * test/node/canSolveEquivalence.ts
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { TBoard } from '../../src/model/board/core/TBoard.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import { CompleteData } from '../../src/model/data/combined/CompleteData.ts';
import { generalEdgeMixedGroup } from '../../src/model/pattern/collection/generalEdgeMixedGroup.ts';
import { generalEdgeSectorMixedGroup } from '../../src/model/pattern/collection/generalEdgeSectorMixedGroup.ts';
import { RuleGroupLimit, compilePatternRules } from '../../src/wasm/compilePatternRules.ts';
import { wasmCanSolve } from '../../src/wasm/slitherCore.ts';
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';

type CanSolveCase = { board: number; difficulty: string; faces: string; result: boolean };
const { boards, cases } = JSON.parse(
  gunzipSync(readFileSync('test/node/fixtures/canSolveCorpus.json.gz')).toString(),
) as {
  boards: { generator: string; parameters: Record<string, unknown> }[];
  cases: CanSolveCase[];
};
const boardObjects: TBoard[] = boards.map((b) =>
  PolygonGeneratorBoard.get(
    polygonGenerators.find((g) => g.name === b.generator)!,
    b.parameters,
  ),
);

const groups: Record<string, RuleGroupLimit[]> = {
  EASY: [{ group: generalEdgeMixedGroup, size: 120 }],
  MEDIUM: [
    { group: generalEdgeSectorMixedGroup, size: 50 },
    { group: generalEdgeMixedGroup, size: 360 },
  ],
};

let compared = 0;
let mismatches = 0;
const start = performance.now();
for (const c of cases) {
  if (!groups[c.difficulty]) {
    continue;
  }
  const board = boardObjects[c.board];
  const state = CompleteData.fromFaces(board, (face) => {
    const character = c.faces[board.faces.indexOf(face)];
    return character === '.' ? null : Number(character);
  });
  const result = wasmCanSolve(board, state, c.difficulty, () => compilePatternRules(board, groups[c.difficulty]));
  compared++;
  if (result !== c.result) {
    mismatches++;
    if (mismatches <= 10) {
      console.log(
        `MISMATCH: ${boards[c.board].generator} ${JSON.stringify(boards[c.board].parameters)} ${c.difficulty}: expected ${c.result}, got ${result}`,
      );
    }
  }
}
console.log(
  `compared ${compared} difficulty checks, ${mismatches} mismatches, ${((performance.now() - start) / 1000).toFixed(1)} s`,
);
process.exit(mismatches ? 1 : 0);
