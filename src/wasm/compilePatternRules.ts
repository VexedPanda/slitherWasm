/**
 * Compiles pattern rules (from the binary rule groups) for one board into flat clauses that the WebAssembly core can
 * apply, using the same feature mapping as BinaryPatternSolver (so the semantics match exactly).
 *
 * For a given board, each (rule, embedding) becomes a clause:
 * - face conditions (the rule's face value features): fixed for a puzzle, so checked once per difficulty check
 * - the board faces the rule covers (for BinaryPatternSolver's separate face check, also fixed for a puzzle)
 * - body atoms: edge black/red and sector flag features that must all hold
 * - head atoms: the output features, set when the body holds
 * - color conditions and color outputs (face color dual features): pairs of faces with the same or opposite colors
 *
 * Atoms are numbered: edge e black = 2e, edge e red = 2e + 1, then for sector (half-edge) h: not zero = 2E + 3h, not
 * one = 2E + 3h + 1, not two = 2E + 3h + 2. "Only one" is "not zero" and "not two".
 *
 * Color relations are (faceA, faceB, kind) with kind 0 for same and 1 for opposite, where face F (the number of faces)
 * is the outside.
 *
 * Each clause is encoded as: faceConditionCount, (face, value)..., faceCount, face..., bodyCount, atom..., headCount,
 * atom..., colorConditionCount, (faceA, faceB, kind)..., colorOutputCount, (faceA, faceB, kind)..., patternFaceCount.
 * A face value of 0xff means "no value".
 */
import { TBoard } from '../model/board/core/TBoard.ts';
import { BinaryMixedRuleGroup } from '../model/pattern/collection/BinaryMixedRuleGroup.ts';
import { getEmbeddings } from '../model/pattern/embedding/getEmbeddings.ts';
import { BlackEdgeFeature } from '../model/pattern/feature/BlackEdgeFeature.ts';
import { getBoardFeaturePositions } from '../model/pattern/feature/BoardFeatureData.ts';
import { FaceFeature } from '../model/pattern/feature/FaceFeature.ts';
import { RedEdgeFeature } from '../model/pattern/feature/RedEdgeFeature.ts';
import { SectorNotOneFeature } from '../model/pattern/feature/SectorNotOneFeature.ts';
import { SectorNotTwoFeature } from '../model/pattern/feature/SectorNotTwoFeature.ts';
import { SectorNotZeroFeature } from '../model/pattern/feature/SectorNotZeroFeature.ts';
import { SectorOnlyOneFeature } from '../model/pattern/feature/SectorOnlyOneFeature.ts';
import { TEmbeddableFeature } from '../model/pattern/feature/TEmbeddableFeature.ts';
import { getBinaryFeatureMapping } from '../model/pattern/generation/BinaryFeatureMapping.ts';
import { BoardPatternBoard } from '../model/pattern/pattern-board/BoardPatternBoard.ts';

export const NO_FACE_VALUE = 0xff;

export type CompiledRules = {
  numSectors: number;
  clauses: Uint32Array;
  clauseCount: number;
};

export type RuleGroupLimit = { group: BinaryMixedRuleGroup; size: number };

export const compilePatternRules = (board: TBoard, groups: RuleGroupLimit[]): CompiledRules => {
  const boardPatternBoard = BoardPatternBoard.get(board);
  const positions = getBoardFeaturePositions(boardPatternBoard);
  const boardEdgeIndices = new Map(board.edges.map((edge, index) => [edge, index]));
  const boardFaceIndices = new Map(board.faces.map((face, index) => [face, index]));
  const halfEdgeIndices = new Map(board.halfEdges.map((halfEdge, index) => [halfEdge, index]));
  const numEdges = board.edges.length;
  const outside = board.faces.length;

  // Pattern-board element index => board index
  const patternEdgeToEdge = positions.edges.map((edge) => boardEdgeIndices.get(edge)!);
  const patternFaceToFace = boardPatternBoard.faces.map((patternFace) => {
    const face = boardPatternBoard.getFace(patternFace);
    return face ? boardFaceIndices.get(face)! : -1;
  });
  const patternSectorToHalfEdge = positions.sectors.map((sector) => halfEdgeIndices.get(sector)!);

  const clauses = new Map<
    string,
    {
      faceConditions: number[];
      faces: number[];
      body: number[];
      head: Set<number>;
      colorConditions: number[];
      colorOutputs: Set<string>;
      patternFaceCount: number;
    }
  >();

  for (const { group, size } of groups) {
    const collection = group.collection;
    for (let ruleIndex = 0; ruleIndex < size; ruleIndex++) {
      const patternBoard = collection.getRulePatternBoard(ruleIndex);
      const mapping = getBinaryFeatureMapping(patternBoard);
      const bytes = collection.getRuleBytes(ruleIndex, false);

      // Split into input and output feature bytes (each section ends with 0xff; face indices in face color dual
      // sections are below 0xfe)
      const separator = bytes.indexOf(0xff);
      const parse = (sectionBytes: number[]) => {
        const features: TEmbeddableFeature[] = [];
        // Face color duals: [main primary face, then (pattern face, isSecondary) pairs]
        const duals: { main: number; others: [number, boolean][] }[] = [];
        for (let i = 0; i < sectionBytes.length; i++) {
          const byte = sectionBytes[i];
          if (byte === 0xfe) {
            const dual = { main: sectionBytes[++i], others: [] as [number, boolean][] };
            while (i + 1 < sectionBytes.length && sectionBytes[i + 1] !== 0xfe) {
              const next = sectionBytes[++i];
              dual.others.push([next & 0x7f, (next & 0x80) !== 0]);
            }
            duals.push(dual);
          } else {
            features.push(mapping.featureArray[byte]);
          }
        }
        return { features, duals };
      };
      const input = parse(bytes.slice(0, separator));
      const output = parse(bytes.slice(separator + 1, bytes.length - 1));

      for (const embedding of getEmbeddings(patternBoard, boardPatternBoard)) {
        const faceConditions: number[] = [];
        const body: number[] = [];
        const head: number[] = [];
        let possible = true;

        const sectorAtom = (patternSectorIndex: number, flag: number) =>
          2 * numEdges + 3 * patternSectorToHalfEdge[embedding.mappedSectorIndices[patternSectorIndex]] + flag;

        // A rule pattern face's board face (the outside for none)
        const mapFace = (patternFaceIndex: number) => {
          const face = patternFaceToFace[embedding.mappedFaceIndices[patternBoard.faces[patternFaceIndex].index]];
          return face === -1 ? outside : face;
        };
        const addColorRelations = (duals: { main: number; others: [number, boolean][] }[], relations: number[]) => {
          for (const dual of duals) {
            const main = mapFace(dual.main);
            for (const [patternFaceIndex, isSecondary] of dual.others) {
              relations.push(main, mapFace(patternFaceIndex), isSecondary ? 1 : 0);
            }
          }
        };
        const colorConditions: number[] = [];
        const colorOutputs: number[] = [];
        addColorRelations(input.duals, colorConditions);
        addColorRelations(output.duals, colorOutputs);

        const addAtoms = (feature: TEmbeddableFeature, atoms: number[], isInput: boolean) => {
          if (feature instanceof FaceFeature) {
            if (!isInput) {
              throw new Error('Face value outputs are not supported');
            }
            const face = patternFaceToFace[embedding.mappedFaceIndices[feature.face.index]];
            const value = feature.value === null ? NO_FACE_VALUE : feature.value;
            if (face === -1) {
              // Outside the board (no value)
              if (value !== NO_FACE_VALUE) {
                possible = false;
              }
            } else {
              faceConditions.push(face, value);
            }
          } else if (feature instanceof BlackEdgeFeature) {
            atoms.push(2 * patternEdgeToEdge[embedding.mappedNonExitEdgeIndices[feature.edge.index]]);
          } else if (feature instanceof RedEdgeFeature) {
            if (feature.edge.isExit) {
              for (const index of embedding.mappedExitEdgeIndices[feature.edge.index]) {
                atoms.push(2 * patternEdgeToEdge[index] + 1);
              }
            } else {
              atoms.push(2 * patternEdgeToEdge[embedding.mappedNonExitEdgeIndices[feature.edge.index]] + 1);
            }
          } else if (feature instanceof SectorNotZeroFeature) {
            atoms.push(sectorAtom(feature.sector.index, 0));
          } else if (feature instanceof SectorNotOneFeature) {
            atoms.push(sectorAtom(feature.sector.index, 1));
          } else if (feature instanceof SectorNotTwoFeature) {
            atoms.push(sectorAtom(feature.sector.index, 2));
          } else if (feature instanceof SectorOnlyOneFeature) {
            atoms.push(sectorAtom(feature.sector.index, 0), sectorAtom(feature.sector.index, 2));
          } else {
            throw new Error(`Unsupported feature: ${feature.constructor.name}`);
          }
        };

        for (const feature of input.features) {
          addAtoms(feature, body, true);
        }
        for (const feature of output.features) {
          addAtoms(feature, head, false);
        }
        if (!possible) {
          continue;
        }

        // The faces the rule covers (including its exit faces), for the separate face check
        const faces = new Set<number>();
        for (const patternFace of patternBoard.faces) {
          const face = patternFaceToFace[embedding.mappedFaceIndices[patternFace.index]];
          if (face !== -1) {
            faces.add(face);
          }
        }

        // Clauses with the same conditions (e.g. from a symmetric rule's equivalent embeddings) are merged, with their
        // outputs combined
        const sortedFaceConditions = sortedTuples(faceConditions, 2);
        const sortedFaces = [...faces].sort((a, b) => a - b);
        const sortedBody = [...new Set(body)].sort((a, b) => a - b);
        const sortedColorConditions = sortedTuples(normalizedRelations(colorConditions), 3);
        const key = [
          sortedFaceConditions.join(','),
          sortedFaces.join(','),
          sortedBody.join(','),
          sortedColorConditions.join(','),
          patternBoard.faces.length,
        ].join('|');
        let clause = clauses.get(key);
        if (!clause) {
          clause = {
            faceConditions: sortedFaceConditions,
            faces: sortedFaces,
            body: sortedBody,
            head: new Set(),
            colorConditions: sortedColorConditions,
            colorOutputs: new Set(),
            patternFaceCount: patternBoard.faces.length,
          };
          clauses.set(key, clause);
        }
        for (const atom of head) {
          clause.head.add(atom);
        }
        const outputs = normalizedRelations(colorOutputs);
        for (let i = 0; i < outputs.length; i += 3) {
          clause.colorOutputs.add(`${outputs[i]},${outputs[i + 1]},${outputs[i + 2]}`);
        }
      }
    }
  }

  const data: number[] = [];
  for (const clause of clauses.values()) {
    const colorOutputs = [...clause.colorOutputs].flatMap((relation) => relation.split(',').map(Number));
    pushAll(data, [clause.faceConditions.length / 2, ...clause.faceConditions]);
    pushAll(data, [clause.faces.length, ...clause.faces]);
    pushAll(data, [clause.body.length, ...clause.body]);
    pushAll(data, [clause.head.size, ...clause.head]);
    pushAll(data, [clause.colorConditions.length / 3, ...clause.colorConditions]);
    pushAll(data, [colorOutputs.length / 3, ...colorOutputs]);
    data.push(clause.patternFaceCount);
  }

  return { numSectors: board.halfEdges.length, clauses: new Uint32Array(data), clauseCount: clauses.size };
};

// (push(...values) can overflow the stack for long arrays)
const pushAll = (target: number[], values: number[]) => {
  for (const value of values) {
    target.push(value);
  }
};

// Flat tuples of the given size, deduplicated and sorted
const sortedTuples = (flat: number[], size: number): number[] => {
  const tuples: number[][] = [];
  for (let i = 0; i < flat.length; i += size) {
    tuples.push(flat.slice(i, i + size));
  }
  const unique = new Map(tuples.map((tuple) => [tuple.join(','), tuple]));
  return [...unique.values()]
    .sort((a, b) => {
      for (let i = 0; i < size; i++) {
        if (a[i] !== b[i]) {
          return a[i] - b[i];
        }
      }
      return 0;
    })
    .flat();
};

// Color relations (faceA, faceB, kind) with faceA <= faceB
const normalizedRelations = (flat: number[]): number[] => {
  const result: number[] = [];
  for (let i = 0; i < flat.length; i += 3) {
    result.push(Math.min(flat[i], flat[i + 1]), Math.max(flat[i], flat[i + 1]), flat[i + 2]);
  }
  return result;
};
