/**
 * Times puzzle generation and tracks peak memory: node test/node/run.mjs test/node/generationBenchmark.ts <size> <difficulty> <count>
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import CanSolveDifficulty from '../../src/model/generator/CanSolveDifficulty.ts';
import { generateAdditiveConstrained } from '../../src/model/generator/generateAdditiveConstrained.ts';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';

const size = Number(process.argv[2] ?? 15);
const difficulty = CanSolveDifficulty.enumeration.getValue(process.argv[3] ?? 'MEDIUM');
const count = Number(process.argv[4] ?? 3);

const board = PolygonGeneratorBoard.get(
  polygonGenerators.find((g) => g.name === 'Square')!,
  {
    width: size,
    height: size,
  },
);

let peak = 0;
const sample = () => {
  peak = Math.max(peak, process.memoryUsage().rss);
};
const timer = setInterval(sample, 50);

const baseline = process.memoryUsage().rss;
const times: number[] = [];
let failures = 0;
for (let i = 0; i < count; i++) {
  const start = performance.now();
  const puzzle = await generateAdditiveConstrained(
    board,
    difficulty,
    new TinyProperty(false),
    new TinyEmitter(),
    new TinyEmitter(),
    new TinyEmitter(),
  );
  times.push(performance.now() - start);
  if (!puzzle) {
    failures++;
  }
  sample();
}
clearInterval(timer);

const mb = (bytes: number) => Math.round(bytes / 1e6);
console.log(
  JSON.stringify({
    size,
    difficulty: process.argv[3] ?? 'MEDIUM',
    seconds: times.map((t) => Math.round(t / 100) / 10),
    failures,
    baselineMB: mb(baseline),
    peakAboveBaselineMB: mb(peak - baseline),
  }),
);
process.exit(0);
