/**
 * Generates puzzles with the WebAssembly generator and checks each is valid: a unique solution (by the original
 * MiniSat-based solver) matching the generator's, and solvable by its difficulty's TS rules (the reference). Also
 * interrupts some generations and resumes them from their progress events. node test/node/run.mjs
 * test/node/generatorValidity.ts [count per case]
 */
import { PolygonGeneratorBoard } from '../../src/model/board/core/PolygonGeneratorBoard.ts';
import { TBoard } from '../../src/model/board/core/TBoard.ts';
import { TStructure } from '../../src/model/board/core/TStructure.ts';
import { polygonGenerators } from '../../src/model/board/generators/polygonGenerators.ts';
import { TCompleteData } from '../../src/model/data/combined/TCompleteData.ts';
import CanSolveDifficulty from '../../src/model/generator/CanSolveDifficulty.ts';
import { GenerationProgressEvent } from '../../src/model/generator/GenerationProgressEvent.ts';
import { TSolvedPuzzle } from '../../src/model/generator/TSolvedPuzzle.ts';
import { generateAdditiveConstrained } from '../../src/model/generator/generateAdditiveConstrained.ts';
import { satSolveMiniSat } from '../../src/model/solver/MiniSatSolver.ts';

import { TinyEmitter, TinyProperty } from 'scenerystack/axon';
import { dotRandom } from 'scenerystack/dot';

const count = Number(process.argv[2] ?? 3);
const MIN_TOUCHED_FRACTION = 0.5;
let failures = 0;

const check = (
  label: string,
  board: TBoard,
  difficulty: CanSolveDifficulty,
  puzzle: TSolvedPuzzle<TStructure, TCompleteData> | null,
) => {
  if (!puzzle) {
    failures++;
    console.log(`FAIL ${label}: no puzzle`);
    return;
  }
  const solutions = satSolveMiniSat(board, puzzle.cleanState, { maxIterations: 10000, failOnMultipleSolutions: false });
  const unique =
    solutions.length === 1 &&
    solutions[0].length === puzzle.blackEdges.size &&
    solutions[0].every((edge) => puzzle.blackEdges.has(edge));
  const solvable = difficulty.tsCanSolve(board, puzzle.cleanState.clone());
  const clues = board.faces.filter((face) => puzzle.cleanState.getFaceValue(face) !== null).length;
  // The loop should wind across the board (not, say, a small loop in a corner): most faces touch it
  const touched = board.faces.filter((face) => face.edges.some((edge) => puzzle.blackEdges.has(edge))).length;
  const spread = touched >= MIN_TOUCHED_FRACTION * board.faces.length;
  if (!unique || !solvable || !spread) {
    failures++;
  }
  console.log(
    `${unique && solvable && spread ? 'ok  ' : 'FAIL'} ${label}: unique ${unique}, solvable ${solvable}, ${clues} clues, loop touches ${touched}/${board.faces.length} faces`,
  );
};

const run = async (
  board: TBoard,
  difficulty: CanSolveDifficulty,
  interruptAfter?: number,
  resume?: GenerationProgressEvent[],
) => {
  const events: GenerationProgressEvent[] = [...(resume ?? [])];
  const interrupted = new TinyProperty(false);
  const define = new TinyEmitter<[number, number | null]>();
  const minimize = new TinyEmitter<[number, number | null]>();
  const reset = new TinyEmitter();
  const record = (event: GenerationProgressEvent) => {
    if (event.type === 'reset') {
      events.length = 0;
    }
    events.push(event);
    if (interruptAfter !== undefined && events.length >= interruptAfter) {
      interrupted.value = true;
    }
  };
  define.addListener((index, state) => record({ type: 'define', index, state }));
  minimize.addListener((index, state) => record({ type: 'minimize', index, state }));
  reset.addListener(() => record({ type: 'reset' }));
  const puzzle = await generateAdditiveConstrained(board, difficulty, interrupted, define, minimize, reset, resume);
  return { puzzle, events };
};

for (const [generatorName, parameters] of [
  ['Square', { width: 10, height: 10 }],
  ['Square', { width: 15, height: 12 }],
  ['Hexagonal', { radius: 4, isPointyTop: true, holeRadius: 0 }],
] as const) {
  const generator = polygonGenerators.find((g) => g.name === generatorName)!;
  const board = PolygonGeneratorBoard.get(generator, { ...generator.defaultParameterValues, ...parameters });
  for (const name of ['NO_LIMIT', 'EASY', 'MEDIUM', 'HARD']) {
    const difficulty = CanSolveDifficulty.enumeration.getValue(name);
    for (let i = 0; i < count; i++) {
      dotRandom.setSeed(1000 * i + name.length);
      const label = `${generatorName} ${JSON.stringify(parameters)} ${name} #${i}`;
      const start = performance.now();
      const { puzzle } = await run(board, difficulty);
      check(`${label} (${((performance.now() - start) / 1000).toFixed(1)} s)`, board, difficulty, puzzle);
    }
    // Interrupt partway (in the minimize phase for most), then resume
    const partial = await run(board, difficulty, board.faces.length + 20);
    const resumed = await run(board, difficulty, undefined, partial.events);
    check(`${generatorName} ${name} resumed after ${partial.events.length} events`, board, difficulty, resumed.puzzle);
  }
}

// A larger board (MEDIUM, the generation the large-board work targets)
{
  const generator = polygonGenerators.find((g) => g.name === 'Square')!;
  const board = PolygonGeneratorBoard.get(generator, { ...generator.defaultParameterValues, width: 30, height: 30 });
  for (let i = 0; i < count; i++) {
    dotRandom.setSeed(1000 * i);
    const start = performance.now();
    const { puzzle } = await run(board, CanSolveDifficulty.MEDIUM);
    check(
      `Square 30x30 MEDIUM #${i} (${((performance.now() - start) / 1000).toFixed(1)} s)`,
      board,
      CanSolveDifficulty.MEDIUM,
      puzzle,
    );
  }
}

console.log(failures ? `${failures} failures` : 'all valid');
process.exit(failures ? 1 : 0);
