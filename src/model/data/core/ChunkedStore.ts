// Values per board element (edge, sector, vertex, face), cheap to clone: values are kept in chunks that clones share,
// and a chunk is only copied when an instance that shares it sets a value in it. States are cloned for every move (and
// kept for undo), so copying a whole map per clone took a lot of memory on large boards.

const CHUNK_BITS = 6;
const CHUNK_SIZE = 1 << CHUNK_BITS;

// Each element's index, per element array (e.g. a board's edges)
const indexMaps = new WeakMap<readonly object[], Map<object, number>>();
const getIndexMap = <K extends object>(elements: readonly K[]): Map<K, number> => {
  let indexMap = indexMaps.get(elements) as Map<K, number> | undefined;
  if (!indexMap) {
    indexMap = new Map(elements.map((element, index) => [element, index]));
    indexMaps.set(elements, indexMap);
  }
  return indexMap;
};

export class ChunkedStore<K extends object, V> {
  private constructor(
    private readonly indexMap: Map<K, number>,
    private readonly chunks: V[][],
    // Whether each chunk is only used by this instance (so can be modified in place)
    private readonly owned: boolean[],
  ) {}

  public static create<K extends object, V>(
    elements: readonly K[],
    getInitialValue: (element: K) => V,
  ): ChunkedStore<K, V> {
    const chunks: V[][] = [];
    for (let start = 0; start < elements.length; start += CHUNK_SIZE) {
      chunks.push(elements.slice(start, start + CHUNK_SIZE).map(getInitialValue));
    }
    return new ChunkedStore(
      getIndexMap(elements),
      chunks,
      chunks.map(() => true),
    );
  }

  public has(element: K): boolean {
    return this.indexMap.has(element);
  }

  public get(element: K): V | undefined {
    const index = this.indexMap.get(element);
    return index === undefined ? undefined : this.chunks[index >> CHUNK_BITS][index & (CHUNK_SIZE - 1)];
  }

  public set(element: K, value: V): void {
    const index = this.indexMap.get(element)!;
    const chunk = index >> CHUNK_BITS;
    if (!this.owned[chunk]) {
      this.chunks[chunk] = this.chunks[chunk].slice();
      this.owned[chunk] = true;
    }
    this.chunks[chunk][index & (CHUNK_SIZE - 1)] = value;
  }

  public clone(): ChunkedStore<K, V> {
    // Both now share every chunk
    this.owned.fill(false);
    return new ChunkedStore(
      this.indexMap,
      this.chunks.slice(),
      this.owned.map(() => false),
    );
  }
}
