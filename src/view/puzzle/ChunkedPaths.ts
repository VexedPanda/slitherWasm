import { Vector2 } from 'scenerystack/dot';
import { Shape } from 'scenerystack/kite';
import { Node, Path, PathOptions } from 'scenerystack/scenery';

// Items (edges, vertices, ...) grouped by position into square cells of this size (in view coordinates)
const CELL_SIZE = 6;

/**
 * Groups a board's items into chunks of nearby items, so a layer drawing them can rebuild only the chunks whose items
 * changed (on a large board, rebuilding and repainting one whole-board shape per move is slow).
 */
export class Chunking {
  public readonly chunkCount: number;
  // The chunk of each item, and each chunk's items
  public readonly itemChunks: number[];
  public readonly chunkItems: number[][];

  public constructor(positions: Vector2[]) {
    const cells = new Map<string, number>();
    this.chunkItems = [];
    this.itemChunks = positions.map((position, index) => {
      const key = `${Math.floor(position.x / CELL_SIZE)},${Math.floor(position.y / CELL_SIZE)}`;
      let chunk = cells.get(key);
      if (chunk === undefined) {
        chunk = this.chunkItems.length;
        cells.set(key, chunk);
        this.chunkItems.push([]);
      }
      this.chunkItems[chunk].push(index);
      return chunk;
    });
    this.chunkCount = this.chunkItems.length;
  }
}

/**
 * One drawn layer (a Path per chunk). Mark the chunks (or items) that changed, then update with how to draw an item.
 */
export class ChunkedPaths extends Node {
  private readonly paths: Path[];
  private readonly dirty: boolean[];

  public constructor(
    private readonly chunking: Chunking,
    pathOptions: PathOptions,
  ) {
    const paths = chunking.chunkItems.map(() => new Path(null, pathOptions));
    super({ children: paths });
    this.paths = paths;
    this.dirty = paths.map(() => true);
  }

  public markItemDirty(item: number): void {
    this.dirty[this.chunking.itemChunks[item]] = true;
  }

  public markAllDirty(): void {
    this.dirty.fill(true);
  }

  // Rebuilds the changed chunks: drawItem adds an item's drawing (if any) to its chunk's shape
  public update(drawItem: (shape: Shape, item: number) => void): void {
    for (let chunk = 0; chunk < this.paths.length; chunk++) {
      if (this.dirty[chunk]) {
        this.dirty[chunk] = false;
        const shape = new Shape();
        for (const item of this.chunking.chunkItems[chunk]) {
          drawItem(shape, item);
        }
        shape.makeImmutable();
        this.paths[chunk].shape = shape;
      }
    }
  }
}
