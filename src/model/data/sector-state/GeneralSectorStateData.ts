import { TBoard } from '../../board/core/TBoard.ts';
import { TSerializedHalfEdge } from '../../board/core/TSerializedHalfEdge.ts';
import { deserializeHalfEdge } from '../../board/core/deserializeHalfEdge.ts';
import { ChunkedStore } from '../core/ChunkedStore.ts';
import { TDelta } from '../core/TDelta.ts';
import { TState } from '../core/TState.ts';
import { GeneralSectorStateDelta } from './GeneralSectorStateDelta.ts';
import SectorState, { TSerializedSectorState } from './SectorState.ts';
import { TSector } from './TSector.ts';
import { TSectorStateData, TSerializedSectorStateData, serializeSectorStateData } from './TSectorStateData.ts';

import { TinyEmitter } from 'scenerystack/axon';

import assert, { assertEnabled } from '../../../workarounds/assert.ts';

export class GeneralSectorStateData implements TState<TSectorStateData> {
  public readonly sectorStateChangedEmitter = new TinyEmitter<
    [sector: TSector, state: SectorState, oldState: SectorState]
  >();

  public readonly sectorStateMap: ChunkedStore<TSector, SectorState>;

  public constructor(
    public readonly board: TBoard,
    getInitialSectorState: (sector: TSector) => SectorState = () => SectorState.ANY,
    // (shared with the state this is cloned from, see ChunkedStore)
    sectorStateMap?: ChunkedStore<TSector, SectorState>,
  ) {
    this.sectorStateMap = sectorStateMap ?? ChunkedStore.create(board.halfEdges, getInitialSectorState);
  }

  public getSectorState(sector: TSector): SectorState {
    assertEnabled() && assert(this.sectorStateMap.has(sector));

    return this.sectorStateMap.get(sector)!;
  }

  public setSectorState(sector: TSector, state: SectorState): void {
    assertEnabled() && assert(this.sectorStateMap.has(sector));

    const oldState = this.sectorStateMap.get(sector)!;

    if (oldState !== state) {
      this.sectorStateMap.set(sector, state);

      this.sectorStateChangedEmitter.emit(sector, state, oldState);
    }
  }

  public clone(): GeneralSectorStateData {
    return new GeneralSectorStateData(
      this.board,
      (element) => this.getSectorState(element),
      this.sectorStateMap.clone(),
    );
  }

  public createDelta(): TDelta<TSectorStateData> {
    return new GeneralSectorStateDelta(this.board, this);
  }

  public serializeState(board: TBoard): TSerializedSectorStateData {
    return serializeSectorStateData(board, this);
  }

  public static deserializeState(
    board: TBoard,
    serializedSectorData: TSerializedSectorStateData,
  ): GeneralSectorStateData {
    const map: Map<TSector, SectorState> = new Map(
      serializedSectorData.sectors.map(
        (serializedSectorState: { sector: TSerializedHalfEdge; state: TSerializedSectorState }) => [
          deserializeHalfEdge(board, serializedSectorState.sector),
          SectorState.deserialize(serializedSectorState.state),
        ],
      ),
    );

    return new GeneralSectorStateData(board, (sector) => map.get(sector) ?? SectorState.ANY);
  }
}
