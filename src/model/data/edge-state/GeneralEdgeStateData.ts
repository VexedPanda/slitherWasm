import { TBoard } from '../../board/core/TBoard.ts';
import { TEdge } from '../../board/core/TEdge.ts';
import { TSerializedEdge } from '../../board/core/TSerializedEdge.ts';
import { deserializeEdge } from '../../board/core/deserializeEdge.ts';
import { ChunkedStore } from '../core/ChunkedStore.ts';
import { TDelta } from '../core/TDelta.ts';
import { TState } from '../core/TState.ts';
import EdgeState from './EdgeState.ts';
import { GeneralEdgeStateDelta } from './GeneralEdgeStateDelta.ts';
import { TEdgeStateData, TSerializedEdgeStateData, serializeEdgeStateData } from './TEdgeStateData.ts';

import { TinyEmitter } from 'scenerystack/axon';

import assert, { assertEnabled } from '../../../workarounds/assert.ts';

// TODO: lots of types like this have duplication, figure out an improvement
export class GeneralEdgeStateData implements TState<TEdgeStateData> {
  public readonly edgeStateChangedEmitter = new TinyEmitter<[edge: TEdge, state: EdgeState, oldState: EdgeState]>();

  public readonly edgeStateMap: ChunkedStore<TEdge, EdgeState>;

  public constructor(
    public readonly board: TBoard,
    getInitialEdgeState: (edge: TEdge) => EdgeState,
    // (shared with the state this is cloned from, see ChunkedStore)
    edgeStateMap?: ChunkedStore<TEdge, EdgeState>,
  ) {
    this.edgeStateMap = edgeStateMap ?? ChunkedStore.create(board.edges, getInitialEdgeState);
  }

  public getEdgeState(edge: TEdge): EdgeState {
    assertEnabled() && assert(this.edgeStateMap.has(edge));

    return this.edgeStateMap.get(edge)!;
  }

  public setEdgeState(edge: TEdge, state: EdgeState): void {
    assertEnabled() && assert(this.edgeStateMap.has(edge));

    const oldState = this.edgeStateMap.get(edge)!;

    if (oldState !== state) {
      this.edgeStateMap.set(edge, state);

      this.edgeStateChangedEmitter.emit(edge, state, oldState);
    }
  }

  public clone(): GeneralEdgeStateData {
    return new GeneralEdgeStateData(this.board, (element) => this.getEdgeState(element), this.edgeStateMap.clone());
  }

  public createDelta(): TDelta<TEdgeStateData> {
    return new GeneralEdgeStateDelta(this.board, this);
  }

  public serializeState(board: TBoard): TSerializedEdgeStateData {
    return serializeEdgeStateData(board, this);
  }

  public static deserializeState(board: TBoard, serializedEdgeData: TSerializedEdgeStateData): GeneralEdgeStateData {
    const map: Map<TEdge, EdgeState> = new Map(
      serializedEdgeData.edges.map((serializedEdgeState: { edge: TSerializedEdge; state: string }) => [
        deserializeEdge(board, serializedEdgeState.edge),
        EdgeState.enumeration.getValue(serializedEdgeState.state),
      ]),
    );

    return new GeneralEdgeStateData(board, (edge) => map.get(edge) ?? EdgeState.WHITE);
  }
}
