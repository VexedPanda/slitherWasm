/**
 * Solves the puzzle using a SAT solver, providing all solutions if helpful.
 *
 * Patterned after the algorithm noted in https://www.dougandjean.com/slither/howitsolves.html (thanks!).
 *
 * Uses https://www.npmjs.com/package/logic-solver (MIT), located under the logic-solver directory (made compatible with ES6 modules).
 *
 * Could in the future consider https://www.comp.nus.edu.sg/~gregory/sat/.
 */
import { TBoard } from '../board/core/TBoard.ts';
import { TEdge } from '../board/core/TEdge.ts';
import { TVertex } from '../board/core/TVertex.ts';
import { TState } from '../data/core/TState.ts';
import EdgeState from '../data/edge-state/EdgeState.ts';
import { TEdgeStateData } from '../data/edge-state/TEdgeStateData.ts';
import { TFaceValueData } from '../data/face-value/TFaceValueData.ts';
import { SatSolveOptions } from './SATSolver.ts';
import { MaximumSolverIterationsError } from './errors/MaximumSolverIterationsError.ts';
import { MultipleSolutionsError } from './errors/MultipleSolutionsError.ts';
// @ts-expect-error
import Logic from './logic-solver/logic-solver.js';

import { Combination } from 'scenerystack/dot';

import assert, { assertEnabled } from '../../workarounds/assert.ts';

/**
 * The original MiniSat-based solver, kept to check the WebAssembly solver against (test/node/solverEquivalence.ts). It
 * also counts some loops that aren't solutions when the given edges are wrong (black edges outside the loop, or faces
 * overfilled by black edges).
 */
// TODO: in the future, vertex state might help! anything that gives us more helpful clauses? maybe not
export const satSolveMiniSat = (
  board: TBoard,
  state: TState<TEdgeStateData & TFaceValueData>,
  options: SatSolveOptions,
): TEdge[][] => {
  // Logic-solver's argument checks are a large part of clause generation time, and our inputs are generated
  return Logic.disablingAssertions(() => satSolveWithoutAssertions(board, state, options));
};

const satSolveWithoutAssertions = (
  board: TBoard,
  state: TState<TEdgeStateData & TFaceValueData>,
  options: SatSolveOptions,
): TEdge[][] => {
  // also, we might as well apply a "normal" solver first, to reduce the number of variables/clauses needed?
  // for instance, this should immediately outlaw 0s

  // TODO: see if outlawing simple loops helps performance later?

  // NOTE: for puzzle generation, holes are either "inside" or "outside" of the puzzle!

  const blackEdges = board.edges.filter((edge) => state.getEdgeState(edge) === EdgeState.BLACK);
  const whiteEdges = board.edges.filter((edge) => state.getEdgeState(edge) === EdgeState.WHITE);

  assertEnabled() && assert(whiteEdges.length, 'Need to have at least some undefined edges to use solver');

  const whiteEdgeSet = new Set(whiteEdges); // For faster lookup

  const edgeToVariable = new Map(whiteEdges.map((edge, i) => [edge, `edge${i + 1}`]));
  const variableToEdge = new Map(whiteEdges.map((edge, i) => [`edge${i + 1}`, edge]));

  const solver = new Logic.Solver();

  // Every constraint below is a plain clause, so we add numeric clauses directly. Going through Logic.or() etc. was
  // most of the solve time (formula objects, flattening, and parsing variable names for every term).
  const edgeToVariableNumber = new Map(whiteEdges.map((edge) => [edge, solver.getVarNum(edgeToVariable.get(edge))]));

  const term = (edge: TEdge): number => {
    const result = edgeToVariableNumber.get(edge);
    assertEnabled() && assert(result !== undefined, 'Edge not found!');
    return result!;
  };
  const notTerm = (edge: TEdge): number => -term(edge);

  const addClause = (terms: number[]) => {
    solver._addClause(new Logic.Clause(terms));
  };

  const none = (edges: TEdge[]) => {
    for (const edge of edges) {
      addClause([notTerm(edge)]);
    }
  };

  const some = (edges: TEdge[]) => {
    addClause(edges.map(term));
  };

  const notAll = (edges: TEdge[]) => {
    addClause(edges.map(notTerm));
  };

  const atLeastN = (edges: TEdge[], n: number) => {
    if (n > 0) {
      some(edges);
      Combination.forEachCombination(edges, (combination: readonly TEdge[]) => {
        if (combination.length === edges.length - n + 1) {
          // NOT( AND( NOT ... ) ), as a plain clause
          addClause(combination.map(term));
        }
      });
    }
  };

  const atMostN = (edges: TEdge[], n: number) => {
    Combination.forEachCombination(edges, (combination: readonly TEdge[]) => {
      if (combination.length === n + 1) {
        addClause(combination.map(notTerm));
      }
    });
  };

  const not1 = (edges: TEdge[]) => {
    // NOT( exactlyOne ) as plain clauses (each edge implies another edge), which avoids the auxiliary variables and
    // formula expansion of Logic.exactlyOne
    for (const edge of edges) {
      addClause([notTerm(edge), ...edges.filter((otherEdge) => otherEdge !== edge).map(term)]);
    }
  };

  const zeroOrTwo = (edges: TEdge[]) => {
    atMostN(edges, 2);
    not1(edges);
  };

  const exactlyN = (edges: TEdge[], n: number) => {
    if (n === 0) {
      none(edges);
    } else {
      atLeastN(edges, n);
      atMostN(edges, n);
    }
  };

  // Add clauses
  some(whiteEdges); // Force there to be at least one edge

  for (const vertex of board.vertices) {
    let blackCount = 0;
    let whiteCount = 0;

    let edges: TEdge[] = [];

    for (const edge of vertex.edges) {
      const edgeState = state.getEdgeState(edge);
      if (edgeState === EdgeState.BLACK) {
        blackCount++;
      } else if (edgeState === EdgeState.WHITE) {
        whiteCount++;
        edges.push(edge);
      }
    }

    if (whiteCount === 0) {
      continue;
    }

    if (blackCount === 0) {
      zeroOrTwo(edges);
    } else if (blackCount === 1) {
      exactlyN(edges, 1);
    } else {
      none(edges);
    }
  }

  let hasFaceWithValue = false;
  for (const face of board.faces) {
    const faceValue = state.getFaceValue(face);
    if (faceValue === null) {
      continue;
    }

    hasFaceWithValue = true;

    let blackCount = 0;
    let whiteCount = 0;

    let edges: TEdge[] = [];

    for (const edge of face.edges) {
      const edgeState = state.getEdgeState(edge);
      if (edgeState === EdgeState.BLACK) {
        blackCount++;
      } else if (edgeState === EdgeState.WHITE) {
        whiteCount++;
        edges.push(edge);
      }
    }

    if (whiteCount === 0) {
      continue;
    }

    const openCount = Math.max(faceValue - blackCount, 0); // sanity check!!!

    exactlyN(edges, openCount);
  }
  assertEnabled() && assert(hasFaceWithValue, 'No faces with values!');

  const getBlackEdges = (): TEdge[] | null => {
    let solution: Logic.Solution | null = null;
    Logic.disablingAssertions(() => {
      solution = solver.solve();
    });

    if (solution) {
      const variables = solution.getTrueVars();
      return variables.map((variable: string) => variableToEdge.get(variable));
    } else {
      return null;
    }
  };

  let iterationNumber = 0;

  // TODO: can we add async/await into minisat somehow? (or web-worker it?)
  const findLoops = (): {
    touchingValueLoops: TEdge[][];
    nonTouchingValueLoops: TEdge[][];
  } => {
    const newBlackEdges = getBlackEdges();
    if (newBlackEdges === null) {
      return {
        touchingValueLoops: [],
        nonTouchingValueLoops: [],
      };
    }

    // Combine the SAT-solved edges with the excluded edges
    const blackEdgeSet = new Set([...newBlackEdges, ...blackEdges]);

    const touchingValueLoops: TEdge[][] = [];
    const nonTouchingValueLoops: TEdge[][] = [];

    while (blackEdgeSet.size) {
      const startEdge = blackEdgeSet.values().next().value;

      const loop: TEdge[] = [];
      let vertex: TVertex = startEdge.end;
      let currentEdge: TEdge | null = startEdge;

      let touchingValue = false;

      while (currentEdge) {
        blackEdgeSet.delete(currentEdge);
        loop.push(currentEdge);

        touchingValue = touchingValue || currentEdge.faces.some((face) => state.getFaceValue(face) !== null);

        currentEdge = vertex.edges.find((e) => blackEdgeSet.has(e)) ?? null;

        if (currentEdge) {
          vertex = currentEdge.getOtherVertex(vertex);
        }
      }

      assertEnabled() &&
        assert(
          loop[0].vertices.some((vertex) => loop[loop.length - 1].vertices.includes(vertex)),
          'Loop is not a loop!',
        );

      (touchingValue ? touchingValueLoops : nonTouchingValueLoops).push(loop);
    }

    return {
      touchingValueLoops,
      nonTouchingValueLoops,
    };
  };

  const solutions: TEdge[][] = [];

  // https://www.dougandjean.com/slither/howitsolves.html describes this partial approach well, thank you!!!

  while (true) {
    const loops = findLoops();

    if (!loops.touchingValueLoops.length) {
      break;
    }

    if (loops.touchingValueLoops.length === 1) {
      solutions.push(loops.touchingValueLoops[0]);

      if (solutions.length > 1 && options.failOnMultipleSolutions) {
        throw new MultipleSolutionsError(solutions);
      }
    }

    const allLoops = [...loops.touchingValueLoops, ...loops.nonTouchingValueLoops];

    for (const loop of allLoops) {
      notAll(loop.filter((edge) => whiteEdgeSet.has(edge)));
    }

    iterationNumber++;

    if (iterationNumber > options.maxIterations) {
      throw new MaximumSolverIterationsError();
    }
  }

  return solutions;
};
