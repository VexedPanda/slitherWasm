import { TBoard } from '../../board/core/TBoard.ts';
import { TFace } from '../../board/core/TFace.ts';
import { TSerializedFace } from '../../board/core/TSerializedFace.ts';
import { deserializeFace } from '../../board/core/deserializeFace.ts';
import { ChunkedStore } from '../core/ChunkedStore.ts';
import { TDelta } from '../core/TDelta.ts';
import { TState } from '../core/TState.ts';
import { FaceState, TSerializedFaceState } from './FaceState.ts';
import { GeneralFaceStateDelta } from './GeneralFaceStateDelta.ts';
import { TFaceStateData, TSerializedFaceStateData, serializeFaceStateData } from './TFaceStateData.ts';

import { TinyEmitter } from 'scenerystack/axon';

import assert, { assertEnabled } from '../../../workarounds/assert.ts';

export class GeneralFaceStateData implements TState<TFaceStateData> {
  public readonly faceStateChangedEmitter = new TinyEmitter<[face: TFace, state: FaceState, oldState: FaceState]>();

  public readonly faceStateMap: ChunkedStore<TFace, FaceState>;

  public constructor(
    public readonly board: TBoard,
    getInitialFaceState: (face: TFace) => FaceState,
    // (shared with the state this is cloned from, see ChunkedStore)
    faceStateMap?: ChunkedStore<TFace, FaceState>,
  ) {
    this.faceStateMap = faceStateMap ?? ChunkedStore.create(board.faces, getInitialFaceState);
  }

  public getFaceState(face: TFace): FaceState {
    assertEnabled() && assert(this.faceStateMap.has(face));

    return this.faceStateMap.get(face)!;
  }

  public setFaceState(face: TFace, state: FaceState): void {
    assertEnabled() && assert(this.faceStateMap.has(face));

    const oldState = this.faceStateMap.get(face)!;

    if (!oldState.equals(state)) {
      this.faceStateMap.set(face, state);

      this.faceStateChangedEmitter.emit(face, state, oldState);
    }
  }

  public clone(): GeneralFaceStateData {
    return new GeneralFaceStateData(this.board, (element) => this.getFaceState(element), this.faceStateMap.clone());
  }

  public createDelta(): TDelta<TFaceStateData> {
    return new GeneralFaceStateDelta(this.board, this);
  }

  public serializeState(board: TBoard): TSerializedFaceStateData {
    return serializeFaceStateData(board, this);
  }

  public static deserializeState(board: TBoard, serializedFaceData: TSerializedFaceStateData): GeneralFaceStateData {
    const map: Map<TFace, FaceState> = new Map(
      serializedFaceData.faces.map((serializedFaceState: { face: TSerializedFace; state: TSerializedFaceState }) => {
        const face = deserializeFace(board, serializedFaceState.face);
        return [face, FaceState.deserialize(face, serializedFaceState.state)];
      }),
    );

    return new GeneralFaceStateData(board, (face) => {
      const faceState = map.get(face)!;
      assertEnabled() && assert(faceState);

      return faceState;
    });
  }
}
