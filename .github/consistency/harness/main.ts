// Generates puzzles with seeded randomness and records a hash of each, plus whether each difficulty's solver can
// solve it. run.mjs copies a checkout's own copy of this into it (as consistency-check/) and serves it with that
// checkout's Vite config, so the base and the changed code each run the harness matching their libraries. Keep the
// generation and hashing the same across versions, or the comparison will report differences.
import { PolygonGeneratorBoard } from '../src/model/board/core/PolygonGeneratorBoard.ts';
import CanSolveDifficulty from '../src/model/generator/CanSolveDifficulty.ts';
import { generateAdditiveConstrained } from '../src/model/generator/generateAdditiveConstrained.ts';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';
import { dotRandom } from 'scenerystack/dot';

type Cases = {
  boards: string[];
  difficulties: string[];
  seeds: number[];
  canSolveDifficulties: string[];
};

const mulberry32 = (seed: number) => () => {
  seed |= 0;
  seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
};

const hash = (s: string) => {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  }
  return (h >>> 0).toString(16).padStart(8, '0');
};

const generate = async (shortName: string, difficultyName: string, seed: number) => {
  const board = PolygonGeneratorBoard.fromShortName(shortName);
  const difficulty = CanSolveDifficulty.enumeration.getValue(difficultyName);

  // Deterministic randomness, so the same seed gives the same puzzle (if generation is unchanged)
  dotRandom.nextDouble = mulberry32(seed);

  return generateAdditiveConstrained(
    board,
    difficulty,
    new TinyProperty(false),
    new TinyEmitter(),
    new TinyEmitter(),
    new TinyEmitter(),
  );
};

(async () => {
  try {
    const cases: Cases = JSON.parse(new URLSearchParams(location.search).get('cases')!);

    // Warm up (loading pattern data, JIT), not recorded
    await generate(cases.boards[0], cases.difficulties[0], 0);

    const results = [];
    for (const board of cases.boards) {
      for (const difficulty of cases.difficulties) {
        for (const seed of cases.seeds) {
          const start = performance.now();
          const puzzle = (await generate(board, difficulty, seed))!;
          const ms = performance.now() - start;

          const canSolveStart = performance.now();
          const canSolve = Object.fromEntries(
            cases.canSolveDifficulties.map((name) => [
              name,
              CanSolveDifficulty.enumeration.getValue(name).canSolve(puzzle.board, puzzle.cleanState.clone()),
            ]),
          );
          const canSolveMs = performance.now() - canSolveStart;

          const result = {
            board,
            difficulty,
            seed,
            // The puzzle itself (clues and solution), not internal ids in its solved state
            hash: hash(
              JSON.stringify({
                faces: puzzle.board.faces.map((face) => puzzle.cleanState.getFaceValue(face)),
                solution: puzzle.board.edges.map((edge) => (puzzle.blackEdges.has(edge) ? 1 : 0)).join(''),
              }),
            ),
            canSolve,
            ms: Math.round(ms),
            canSolveMs: Math.round(canSolveMs),
          };
          results.push(result);
          console.log(`[consistency] ${JSON.stringify(result)}`);
        }
      }
    }

    // @ts-expect-error
    window.consistencyResult = results;
  } catch (e) {
    // @ts-expect-error
    window.consistencyError = e instanceof Error ? (e.stack ?? e.message) : String(e);
  }
})();
