import { TBoard } from '../../board/core/TBoard.ts';
import { TFace } from '../../board/core/TFace.ts';
import { deserializeFace } from '../../board/core/deserializeFace.ts';
import { ChunkedStore } from '../core/ChunkedStore.ts';
import { TDelta } from '../core/TDelta.ts';
import { TState } from '../core/TState.ts';
import { GeneralFaceColor } from './GeneralFaceColor.ts';
import { GeneralFaceColorDelta } from './GeneralFaceColorDelta.ts';
import FaceColorState, {
  TFaceColor,
  TFaceColorData,
  TSerializedFaceColorData,
  serializeFaceColorData,
} from './TFaceColorData.ts';

import { TinyEmitter } from 'scenerystack/axon';

import { MultiIterable } from '../../../workarounds/MultiIterable.ts';
import assert, { assertEnabled } from '../../../workarounds/assert.ts';

// Random (to be unique across saved and restored states), but not from dotRandom: puzzle generation uses dotRandom, so
// it would then depend on how many face colors its difficulty checks happen to create.
export const getFaceColorGlobalId = (): number => Math.floor(Math.random() * Number.MAX_SAFE_INTEGER);

export class GeneralFaceColorData implements TState<TFaceColorData> {
  public readonly faceColorsChangedEmitter = new TinyEmitter<
    [
      addedFaceColors: MultiIterable<TFaceColor>,
      removedFaceColors: MultiIterable<TFaceColor>,
      oppositeChangedFaceColors: MultiIterable<TFaceColor>,
      changedFaces: MultiIterable<TFace>,
    ]
  >();

  // Each face's color (shared with clones, see ChunkedStore)
  public readonly colorMap: ChunkedStore<TFace, TFaceColor>;
  // Each color's faces (its keys are the face colors), and opposite colors: shared with clones until modified (see
  // ownsCollections)
  public colorInverseMap: Map<TFaceColor, Set<TFace>>;
  public oppositeColorMap: Map<TFaceColor, TFaceColor | null>;
  public readonly outsideColor: TFaceColor;
  public readonly insideColor: TFaceColor;
  public invalidFaceColor: boolean;

  // colorInverseMap's face sets are shared between clones (copying one per face color for every clone, e.g. every undo
  // step, takes a lot of memory on large boards): only these, created by this instance since it was last cloned, may be
  // modified in place, and any other is copied first
  private readonly ownedFaceSets = new Set<Set<TFace>>();

  // Whether the collections above are only used by this instance (otherwise they're copied before being modified)
  private ownsCollections: boolean;

  public constructor(
    public readonly board: TBoard,
    // If not provided, we'll assume that the face colors are the same as the faces on the board.
    faceColors?: Set<TFaceColor>,
    colorMap?: Map<TFace, TFaceColor> | ChunkedStore<TFace, TFaceColor>,
    colorInverseMap?: Map<TFaceColor, Set<TFace>>,
    oppositeColorMap?: Map<TFaceColor, TFaceColor | null>,
    outsideColor?: TFaceColor,
    insideColor?: TFaceColor,
    invalidFaceColor?: boolean,
  ) {
    // TODO: use a better pattern here
    assertEnabled() && assert(!faceColors || insideColor, 'Provide all or none of the optional arguments');

    this.outsideColor = outsideColor || new GeneralFaceColor(getFaceColorGlobalId(), FaceColorState.OUTSIDE);
    this.insideColor = insideColor || new GeneralFaceColor(getFaceColorGlobalId(), FaceColorState.INSIDE);

    this.ownsCollections = !colorInverseMap;

    if (colorMap instanceof ChunkedStore) {
      this.colorMap = colorMap;
    } else if (colorMap) {
      this.colorMap = ChunkedStore.create(board.faces, (face) => colorMap.get(face)!);
    } else {
      this.colorMap = ChunkedStore.create(
        board.faces,
        () => new GeneralFaceColor(getFaceColorGlobalId(), FaceColorState.UNDECIDED),
      );
    }

    if (colorInverseMap) {
      this.colorInverseMap = colorInverseMap;
    } else {
      const colors = faceColors ?? [
        this.outsideColor,
        this.insideColor,
        ...board.faces.map((face) => this.colorMap.get(face)!),
      ];
      this.colorInverseMap = new Map([...colors].map((faceColor) => [faceColor, new Set<TFace>()]));
      for (const face of board.faces) {
        this.colorInverseMap.get(this.colorMap.get(face)!)!.add(face);
      }
      for (const faces of this.colorInverseMap.values()) {
        this.ownedFaceSets.add(faces);
      }
    }

    this.oppositeColorMap =
      oppositeColorMap ??
      new Map([
        [this.outsideColor, this.insideColor],
        [this.insideColor, this.outsideColor],
      ]);

    this.invalidFaceColor = !!invalidFaceColor;

    assertEnabled() && assert(this.board.faces.every((face) => this.colorInverseMap.has(this.colorMap.get(face)!)));
  }

  public getFaceColors(): TFaceColor[] {
    // TODO: better perf way to handle this?
    return [...this.colorInverseMap.keys()];
  }

  public getInsideColor(): TFaceColor {
    return this.insideColor;
  }

  public getOutsideColor(): TFaceColor {
    return this.outsideColor;
  }

  public getFaceColor(face: TFace): TFaceColor {
    const faceColor = this.colorMap.get(face)!;
    assertEnabled() && assert(faceColor);

    return faceColor;
  }

  public getFacesWithColor(faceColor: TFaceColor): TFace[] {
    assertEnabled() && assert(this.colorInverseMap.has(faceColor));

    const faces = this.colorInverseMap.get(faceColor)!;
    assertEnabled() && assert(faces);

    // TODO: better perf way to handle this?
    return [...faces];
  }

  public getFaceColorMap(): Map<TFace, TFaceColor> {
    return new Map(this.board.faces.map((face) => [face, this.colorMap.get(face)!]));
  }

  public getOppositeFaceColor(faceColor: TFaceColor): TFaceColor | null {
    return this.oppositeColorMap.get(faceColor) ?? null;
  }

  public hasInvalidFaceColors(): boolean {
    return this.invalidFaceColor;
  }

  // The faces with a color, to modify in place (see ownedFaceSets)
  private getMutableFacesWithColor(faceColor: TFaceColor): Set<TFace> {
    let faces = this.colorInverseMap.get(faceColor)!;
    if (!this.ownedFaceSets.has(faces)) {
      faces = new Set(faces);
      this.colorInverseMap.set(faceColor, faces);
      this.ownedFaceSets.add(faces);
    }
    return faces;
  }

  public modifyFaceColors(
    addedFaceColors: MultiIterable<TFaceColor>,
    removedFaceColors: MultiIterable<TFaceColor>,
    faceChangeMap: Map<TFace, TFaceColor>,
    oppositeChangeMap: Map<TFaceColor, TFaceColor | null>,
    invalidFaceColor: boolean,
  ): void {
    if (!this.ownsCollections) {
      this.ownsCollections = true;
      this.colorInverseMap = new Map(this.colorInverseMap);
      this.oppositeColorMap = new Map(this.oppositeColorMap);
    }

    for (const addedFaceColor of addedFaceColors) {
      const faces = new Set<TFace>();
      this.colorInverseMap.set(addedFaceColor, faces);
      this.ownedFaceSets.add(faces);
    }

    for (const [face, newColor] of faceChangeMap.entries()) {
      const oldColor = this.colorMap.get(face)!;
      assertEnabled() && assert(oldColor);

      this.colorMap.set(face, newColor);
      this.getMutableFacesWithColor(oldColor).delete(face);
      this.getMutableFacesWithColor(newColor).add(face);
    }

    const colorsToRemove = new Set<TFaceColor>();
    for (const removedFaceColor of removedFaceColors) {
      colorsToRemove.add(removedFaceColor);

      const removedFaces = this.colorInverseMap.get(removedFaceColor);
      if (removedFaces) {
        this.ownedFaceSets.delete(removedFaces);
      }
      this.colorInverseMap.delete(removedFaceColor);
      this.oppositeColorMap.delete(removedFaceColor);
    }

    for (const [oldColor, newColor] of oppositeChangeMap.entries()) {
      if (newColor === null) {
        this.oppositeColorMap.delete(oldColor);
      } else if (!colorsToRemove.has(oldColor)) {
        // If we have the start of a "removed" color, don't set the rest
        this.oppositeColorMap.set(oldColor, newColor);
        this.oppositeColorMap.set(newColor, oldColor);
      }
    }

    // Remove opposite colors that are now invalid
    for (const color of this.colorInverseMap.keys()) {
      if (this.oppositeColorMap.has(color) && colorsToRemove.has(this.oppositeColorMap.get(color)!)) {
        this.oppositeColorMap.delete(color);
      }
    }

    const oppositeChangedFaceColors = new Set<TFaceColor>(oppositeChangeMap.keys());

    this.invalidFaceColor = invalidFaceColor;

    assertEnabled() && assert(this.board.faces.every((face) => this.colorInverseMap.has(this.colorMap.get(face)!)));

    // Lots of error checks for cases we're having issues with
    if (assertEnabled()) {
      const colors = new Set(this.getFaceColors());

      for (const color of colors) {
        const opposite = this.getOppositeFaceColor(color);
        if (opposite && !colors.has(opposite)) {
          assert(false, `opposite color ${opposite} of color ${color} is not in the set of colors`);
        }
      }

      for (const color of this.oppositeColorMap.keys()) {
        assert(this.oppositeColorMap.has(this.oppositeColorMap.get(color)!));
      }
    }

    this.faceColorsChangedEmitter.emit(addedFaceColors, removedFaceColors, oppositeChangedFaceColors, [
      ...faceChangeMap.keys(),
    ]);
  }

  public clone(): GeneralFaceColorData {
    // The collections and face sets are now shared with the clone
    this.ownsCollections = false;
    this.ownedFaceSets.clear();
    return new GeneralFaceColorData(
      this.board,
      undefined,
      this.colorMap.clone(),
      this.colorInverseMap,
      this.oppositeColorMap,
      this.outsideColor,
      this.insideColor,
      this.invalidFaceColor,
    );
  }

  public createDelta(): TDelta<TFaceColorData> {
    return new GeneralFaceColorDelta(this.board, this);
  }

  public serializeState(board: TBoard): TSerializedFaceColorData {
    return serializeFaceColorData(this);
  }

  public static deserializeState(
    board: TBoard,
    serializedFaceColorData: TSerializedFaceColorData,
  ): GeneralFaceColorData {
    const colors = serializedFaceColorData.colors.map((serializedFaceColor) => {
      const id = serializedFaceColor.id;
      const colorState = FaceColorState.enumeration.getValue(serializedFaceColor.colorState)!;
      assertEnabled() && assert(colorState);

      return new GeneralFaceColor(id, colorState);
    });

    const colorMap = new Map<TFace, TFaceColor>();
    const colorInverseMap = new Map<TFaceColor, Set<TFace>>();
    const oppositeColorMap = new Map<TFaceColor, TFaceColor>();

    colors.forEach((color, i) => {
      const faces = serializedFaceColorData.colors[i].faces.map((serializedFace) =>
        deserializeFace(board, serializedFace),
      );
      faces.forEach((face) => colorMap.set(face, color));
      colorInverseMap.set(color, new Set(faces));

      const oppositeColorId = serializedFaceColorData.colors[i].oppositeFaceColorId;

      if (oppositeColorId !== null) {
        const oppositeColor = colors.find((color) => color.id === oppositeColorId)!;
        assertEnabled() && assert(oppositeColor);

        oppositeColorMap.set(color, oppositeColor);
      }
    });

    const outsideColor = colors.find((color) => color.colorState === FaceColorState.OUTSIDE)!;
    assertEnabled() && assert(outsideColor);

    const insideColor = colors.find((color) => color.colorState === FaceColorState.INSIDE)!;
    assertEnabled() && assert(insideColor);

    return new GeneralFaceColorData(
      board,
      new Set(colors),
      colorMap,
      colorInverseMap,
      oppositeColorMap,
      outsideColor,
      insideColor,
      serializedFaceColorData.invalidFaceColor,
    );
  }
}
