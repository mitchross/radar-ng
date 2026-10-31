/**
 * The decoded-tile cache behind the in-map radar: LRU by Map order, a byte budget, a bounded
 * number of loads in flight, and a retry for failed tiles. No React and no Skia: `T` is whatever
 * the loader decodes a tile into, and the clock and loader are injected.
 */

export type Entry<T> =
  | { state: "loading" }
  | { state: "empty" }
  | { state: "error"; at: number }
  | { state: "ready"; handle: T; height: number; bytes: number };

export type LoadResult<T> =
  | { kind: "empty" }
  | { kind: "error" }
  | { kind: "ready"; handle: T; height: number; bytes: number };

export interface TileCacheOptions<T> {
  /** Decoded bytes kept in memory. */
  budgetBytes: number;
  maxInFlight: number;
  errorRetryMs: number;
  /** Also bounds the small 204/error entries that every bucket version adds. */
  maxEntries: number;
  now: () => number;
  /** A rejection counts as a failed tile. */
  load: (url: string) => Promise<LoadResult<T>>;
}

export function createTileCache<T>(opts: TileCacheOptions<T>) {
  const { budgetBytes, maxInFlight, errorRetryMs, maxEntries, now, load } = opts;

  /** Map order = LRU order. */
  const cache = new Map<string, Entry<T>>();
  /** layer/bucket/z/x/y → URL of the newest ready version, for showing an older `v` while a new one loads. */
  const newestReady = new Map<string, string>();
  let cachedBytes = 0;
  let wanted: string[] = [];
  let wantedSet = new Set<string>();
  /** Wanted URL → its newestReady key. */
  let wantedKeys = new Map<string, string>();
  let inFlight = 0;
  const listeners = new Set<() => void>();

  function evict() {
    for (const [url, entry] of cache) {
      if (cachedBytes <= budgetBytes && cache.size <= maxEntries) return;
      if (entry.state === "loading" || wantedSet.has(url)) continue;
      if (entry.state === "ready") {
        if (cachedBytes <= budgetBytes) continue;
        cachedBytes -= entry.bytes;
      }
      cache.delete(url);
    }
  }

  async function run(url: string, key: string) {
    inFlight++;
    try {
      const result = await load(url);
      if (result.kind === "empty") {
        cache.set(url, { state: "empty" });
      } else if (result.kind === "error") {
        cache.set(url, { state: "error", at: now() });
      } else {
        cache.set(url, { state: "ready", handle: result.handle, height: result.height, bytes: result.bytes });
        newestReady.set(key, url);
        cachedBytes += result.bytes;
      }
    } catch {
      cache.set(url, { state: "error", at: now() });
    } finally {
      inFlight--;
      evict();
      listeners.forEach((l) => l());
      pump();
    }
  }

  /** Start wanted loads in priority order, at most maxInFlight at a time. */
  function pump() {
    for (const url of wanted) {
      if (inFlight >= maxInFlight) return;
      const entry = cache.get(url);
      if (entry && !(entry.state === "error" && now() - entry.at > errorRetryMs)) continue;
      cache.set(url, { state: "loading" });
      void run(url, wantedKeys.get(url) ?? url);
    }
  }

  return {
    get: (url: string) => cache.get(url),
    /** Mark an entry most recently used. */
    touch(url: string, entry: Entry<T>) {
      cache.delete(url);
      cache.set(url, entry);
    },
    newestReadyUrl: (key: string) => newestReady.get(key),
    /** Replace the wanted set (priority order) and start loading it. */
    setWanted(urls: string[], keys: Map<string, string>) {
      wanted = urls;
      wantedSet = new Set(urls);
      wantedKeys = keys;
      pump();
    },
    /** Called after every finished load; returns the unsubscribe. */
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    stats: () => ({ bytes: cachedBytes, entries: cache.size, inFlight }),
  };
}
