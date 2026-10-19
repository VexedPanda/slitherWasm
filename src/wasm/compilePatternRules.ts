/**
 * Compiles pattern rules (from the binary rule groups) for one board into flat clauses that the WebAssembly core can
 * apply, using the same feature mapping as BinaryPatternSolver (so the semantics match exactly).
 *
 * For a given board, each (rule, embedding) becomes a clause:
 * - face conditions (the rule's face value features): fixed for a puzzle, so checked once per difficulty check
 * - the board faces the rule covers (for BinaryPatternSolver's separate face check, also fixed for a puzzle)
 * - body atoms: edge black/red and sector flag features that must all hold
 * - head atoms: the output features, set when the body holds
 *
 * Atoms are numbered: edge e black = 2e, edge e red = 2e + 1, then for pattern-board sector s: not zero = 2E + 3s,
 * not one = 2E + 3s + 1, not two = 2E + 3s + 2. "Only one" is "not zero" and "not two".
 *
 * Each clause is encoded as: faceConditionCount, (face, value)..., faceCount, face..., bodyCount, atom..., headCount,
 * atom..., patternFaceCount. A face value of 0xff means "no value".
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
  const numEdges = board.edges.length;

  // Pattern-board element index => board index
  const patternEdgeToEdge = positions.edges.map((edge) => boardEdgeIndices.get(edge)!);
  const patternFaceToFace = boardPatternBoard.faces.map((patternFace) => {
    const face = boardPatternBoard.getFace(patternFace);
    return face ? boardFaceIndices.get(face)! : -1;
  });

  const data: number[] = [];
  let clauseCount = 0;

  for (const { group, size } of groups) {
    const collection = group.collection;
    for (let ruleIndex = 0; ruleIndex < size; ruleIndex++) {
      const patternBoard = collection.getRulePatternBoard(ruleIndex);
      const mapping = getBinaryFeatureMapping(patternBoard);
      const bytes = collection.getRuleBytes(ruleIndex, false);

      // Split into input and output feature bytes (each section ends with 0xff)
      const separator = bytes.indexOf(0xff);
      const inputBytes = bytes.slice(0, separator);
      const outputBytes = bytes.slice(separator + 1, bytes.length - 1);
      if (inputBytes.includes(0xfe) || outputBytes.includes(0xfe)) {
        throw new Error('Face color rules are not supported yet');
      }
      const inputFeatures = inputBytes.map((byte) => mapping.featureArray[byte]);
      const outputFeatures = outputBytes.map((byte) => mapping.featureArray[byte]);

      for (const embedding of getEmbeddings(patternBoard, boardPatternBoard)) {
        const faceConditions: number[] = [];
        const body: number[] = [];
        const head: number[] = [];
        let possible = true;

        const sectorAtom = (patternSectorIndex: number, flag: number) =>
          2 * numEdges + 3 * embedding.mappedSectorIndices[patternSectorIndex] + flag;

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

        for (const feature of inputFeatures) {
          addAtoms(feature, body, true);
        }
        for (const feature of outputFeatures) {
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

        data.push(faceConditions.length / 2, ...faceConditions);
        data.push(faces.size, ...faces);
        data.push(body.length, ...body);
        data.push(head.length, ...head);
        data.push(patternBoard.faces.length);
        clauseCount++;
      }
    }
  }

  return { numSectors: positions.sectors.length, clauses: new Uint32Array(data), clauseCount };
};
