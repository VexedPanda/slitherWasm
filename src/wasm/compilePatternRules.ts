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
 * Color relations are (faceA, faceB, kind) with faceA <= faceB, and kind 0 for same and 1 for opposite, where face F (the
 * number of faces) is the outside.
 *
 * Each clause is encoded as: faceConditionCount, (face, value)..., faceCount, face..., bodyCount, atom..., headCount,
 * atom..., colorConditionCount, (faceA, faceB, kind)..., colorOutputCount, (faceA, faceB, kind)..., patternFaceCount,
 * with each list sorted (so equal conditions are equal sequences). A face value of 0xff means "no value". Clauses with
 * the same conditions (e.g. from a symmetric rule's equivalent embeddings) are merged by the core (cansolve.rs).
 *
 * Large boards have hundreds of thousands of clauses, so (for memory, e.g. on phones) they're written straight into a
 * typed array, and each pattern board's embeddings are only kept while its rules are compiled.
 */
import { TBoard } from '../model/board/core/TBoard.ts';
import { BinaryMixedRuleGroup } from '../model/pattern/collection/BinaryMixedRuleGroup.ts';
import { computeEmbeddings } from '../model/pattern/embedding/computeEmbeddings.ts';
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
import { TPatternBoard } from '../model/pattern/pattern-board/TPatternBoard.ts';

export const NO_FACE_VALUE = 0xff;

export type CompiledRules = {
  numSectors: number;
  clauses: Uint32Array;
  clauseCount: number;
};

// The first `size` rules of a group, optionally without its highlander rules (which assume the puzzle is unique)
export type RuleGroupLimit = { group: BinaryMixedRuleGroup; size: number; withoutHighlander?: boolean };

// Face color dual: the main primary face, then (pattern face, isSecondary) pairs
type Dual = { main: number; others: [number, boolean][] };

type ParsedRule = {
  patternBoard: TPatternBoard;
  inputFeatures: TEmbeddableFeature[];
  outputFeatures: TEmbeddableFeature[];
  inputDuals: Dual[];
  outputDuals: Dual[];
};

// A growable Uint32Array
class Output {
  public data = new Uint32Array(1 << 16);
  public length = 0;

  public push(value: number): void {
    if (this.length === this.data.length) {
      const data = new Uint32Array(this.data.length * 2);
      data.set(this.data);
      this.data = data;
    }
    this.data[this.length++] = value;
  }

  // The number of tuples, then the flat values
  public pushTuples(flat: number[], tupleSize: number): void {
    this.push(flat.length / tupleSize);
    for (const value of flat) {
      this.push(value);
    }
  }
}

const parseRule = (group: BinaryMixedRuleGroup, ruleIndex: number): ParsedRule => {
  const collection = group.collection;
  const patternBoard = collection.getRulePatternBoard(ruleIndex);
  const mapping = getBinaryFeatureMapping(patternBoard);
  const bytes = collection.getRuleBytes(ruleIndex, false);

  // Input and output feature bytes (each section ends with 0xff; face indices in face color dual sections are below
  // 0xfe)
  const parse = (sectionBytes: number[]) => {
    const features: TEmbeddableFeature[] = [];
    const duals: Dual[] = [];
    for (let i = 0; i < sectionBytes.length; i++) {
      const byte = sectionBytes[i];
      if (byte === 0xfe) {
        const dual: Dual = { main: sectionBytes[++i], others: [] };
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
  const separator = bytes.indexOf(0xff);
  const input = parse(bytes.slice(0, separator));
  const output = parse(bytes.slice(separator + 1, bytes.length - 1));
  return {
    patternBoard,
    inputFeatures: input.features,
    outputFeatures: output.features,
    inputDuals: input.duals,
    outputDuals: output.duals,
  };
};

const sortedUnique = (values: number[]): number[] => [...new Set(values)].sort((a, b) => a - b);

// Flat tuples of the given size, deduplicated and sorted
const sortedTuples = (flat: number[], size: number): number[] => {
  const tuples: number[][] = [];
  for (let i = 0; i < flat.length; i += size) {
    const tuple = flat.slice(i, i + size);
    if (!tuples.some((other) => other.every((value, j) => value === tuple[j]))) {
      tuples.push(tuple);
    }
  }
  return tuples
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

  // Rules grouped by pattern board, so each pattern board's embeddings are computed once (and then released)
  const rulesByPatternBoard = new Map<TPatternBoard, ParsedRule[]>();
  for (const { group, size, withoutHighlander } of groups) {
    for (let ruleIndex = 0; ruleIndex < size; ruleIndex++) {
      if (withoutHighlander && group.isRuleIndexHighlander(ruleIndex)) {
        continue;
      }
      const rule = parseRule(group, ruleIndex);
      let rules = rulesByPatternBoard.get(rule.patternBoard);
      if (!rules) {
        rules = [];
        rulesByPatternBoard.set(rule.patternBoard, rules);
      }
      rules.push(rule);
    }
  }

  const output = new Output();
  let clauseCount = 0;

  for (const [patternBoard, rules] of rulesByPatternBoard) {
    for (const embedding of computeEmbeddings(patternBoard, boardPatternBoard)) {
      const sectorAtom = (patternSectorIndex: number, flag: number) =>
        2 * numEdges + 3 * patternSectorToHalfEdge[embedding.mappedSectorIndices[patternSectorIndex]] + flag;

      // A rule pattern face's board face (the outside for none)
      const mapFace = (patternFaceIndex: number) => {
        const face = patternFaceToFace[embedding.mappedFaceIndices[patternBoard.faces[patternFaceIndex].index]];
        return face === -1 ? outside : face;
      };

      // The faces the rules cover (including their exit faces), for the separate face check
      const faces: number[] = [];
      for (const patternFace of patternBoard.faces) {
        const face = patternFaceToFace[embedding.mappedFaceIndices[patternFace.index]];
        if (face !== -1) {
          faces.push(face);
        }
      }
      const sortedFaces = sortedUnique(faces);

      // Face color relations, normalized to (smaller face, larger face, kind)
      const relations = (duals: Dual[]) => {
        const result: number[] = [];
        for (const dual of duals) {
          const main = mapFace(dual.main);
          for (const [patternFaceIndex, isSecondary] of dual.others) {
            const other = mapFace(patternFaceIndex);
            result.push(Math.min(main, other), Math.max(main, other), isSecondary ? 1 : 0);
          }
        }
        return sortedTuples(result, 3);
      };

      for (const rule of rules) {
        const faceConditions: number[] = [];
        const body: number[] = [];
        const head: number[] = [];
        let possible = true;

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

        for (const feature of rule.inputFeatures) {
          addAtoms(feature, body, true);
        }
        for (const feature of rule.outputFeatures) {
          addAtoms(feature, head, false);
        }
        if (!possible) {
          continue;
        }

        output.pushTuples(sortedTuples(faceConditions, 2), 2);
        output.pushTuples(sortedFaces, 1);
        output.pushTuples(sortedUnique(body), 1);
        output.pushTuples(sortedUnique(head), 1);
        output.pushTuples(relations(rule.inputDuals), 3);
        output.pushTuples(relations(rule.outputDuals), 3);
        output.push(patternBoard.faces.length);
        clauseCount++;
      }
    }
  }

  return { numSectors: board.halfEdges.length, clauses: output.data.slice(0, output.length), clauseCount };
};
