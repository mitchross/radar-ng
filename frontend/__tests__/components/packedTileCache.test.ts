import { createTileCache, type LoadResult } from "../../src/components/map/packedTileCache";

type Handle = { id: string };

/** A loader whose requests stay pending until the test settles them. */
function setup(overrides: { budgetBytes?: number; maxInFlight?: number; maxEntries?: number } = {}) {
  let clock = 1_000;
  const pending = new Map<string, { resolve: (r: LoadResult<Handle>) => void; reject: (e: Error) => void }>();
  const started: string[] = [];
  const cache = createTileCache<Handle>({
    budgetBytes: 100,
    maxInFlight: 6,
    errorRetryMs: 30_000,
    maxEntries: 2000,
    ...overrides,
    now: () => clock,
    load: (url) => {
      started.push(url);
      return new Promise((resolve, reject) => pending.set(url, { resolve, reject }));
    },
  });
  const ready = (url: string, bytes = 40): LoadResult<Handle> => ({
    kind: "ready",
    handle: { id: url },
    height: 512,
    bytes,
  });
  /** Let the cache's awaited load settle and its finally block run. */
  const flush = async () => {
    for (let i = 0; i < 5; i++) await Promise.resolve();
  };
  const settle = async (url: string, result: LoadResult<Handle>) => {
    pending.get(url)!.resolve(result);
    pending.delete(url);
    await flush();
  };
  const wantNow = (...urls: string[]) => cache.setWanted(urls, new Map(urls.map((u) => [u, `key:${u}`])));
  return {
    cache,
    started,
    pending,
    ready,
    settle,
    flush,
    wantNow,
    advance: (ms: number) => {
      clock += ms;
    },
    fail: async (url: string) => {
      pending.get(url)!.reject(new Error("network"));
      pending.delete(url);
      await flush();
    },
  };
}

describe("tile cache eviction", () => {
  it("drops the least recently used unwanted tile first when over budget", async () => {
    const t = setup();
    t.wantNow("a", "b");
    await t.settle("a", t.ready("a"));
    await t.settle("b", t.ready("b"));
    expect(t.cache.stats().bytes).toBe(80);

    // Drawing `a` makes it the most recent; `b` is now the oldest.
    t.cache.touch("a", t.cache.get("a")!);
    t.wantNow("c");
    await t.settle("c", t.ready("c"));

    expect(t.cache.get("b")).toBeUndefined();
    expect(t.cache.get("a")?.state).toBe("ready");
    expect(t.cache.get("c")?.state).toBe("ready");
    expect(t.cache.stats().bytes).toBe(80);
  });

  it("stops evicting as soon as the cache is back under budget", async () => {
    const t = setup();
    t.wantNow("a", "b");
    await t.settle("a", t.ready("a"));
    await t.settle("b", t.ready("b"));
    t.wantNow("c");
    await t.settle("c", t.ready("c")); // 120 > 100: only the oldest (a) has to go

    expect(t.cache.get("a")).toBeUndefined();
    expect(t.cache.get("b")?.state).toBe("ready");
    expect(t.cache.get("c")?.state).toBe("ready");
  });

  it("never evicts a wanted tile, even when over budget", async () => {
    const t = setup();
    t.wantNow("a", "b", "c");
    await t.settle("a", t.ready("a"));
    await t.settle("b", t.ready("b"));
    await t.settle("c", t.ready("c"));

    expect(t.cache.stats().bytes).toBe(120);
    expect(["a", "b", "c"].map((u) => t.cache.get(u)?.state)).toEqual(["ready", "ready", "ready"]);
  });

  it("never evicts a tile that is still loading", async () => {
    const t = setup({ budgetBytes: 10, maxEntries: 1 });
    t.wantNow("a");
    await t.settle("a", t.ready("a", 40));
    t.wantNow("b", "c");
    await t.settle("b", t.ready("b", 40)); // `c` is still loading while the cache is over budget

    expect(t.cache.get("c")?.state).toBe("loading");
    expect(t.cache.get("a")).toBeUndefined();
    expect(t.cache.get("b")?.state).toBe("ready");
  });

  it("keeps ready tiles that fit the byte budget but sheds empty/error entries past the entry cap", async () => {
    const t = setup({ maxEntries: 2 });
    t.wantNow("a", "b", "c");
    await t.settle("a", { kind: "empty" });
    await t.settle("b", t.ready("b"));
    await t.settle("c", { kind: "error" });
    t.wantNow("d");
    await t.settle("d", t.ready("d")); // 4 entries, 80 bytes: over the cap, under budget

    expect(t.cache.get("a")).toBeUndefined(); // empty, oldest
    expect(t.cache.get("b")?.state).toBe("ready"); // ready tiles are only dropped for bytes
    expect(t.cache.get("c")).toBeUndefined(); // error
    expect(t.cache.get("d")?.state).toBe("ready");
    expect(t.cache.stats().bytes).toBe(80);
  });

  it("remembers the newest ready URL per tile key", async () => {
    const t = setup();
    t.wantNow("a");
    await t.settle("a", t.ready("a"));
    expect(t.cache.newestReadyUrl("key:a")).toBe("a");
    expect(t.cache.newestReadyUrl("key:zzz")).toBeUndefined();
  });
});

describe("tile cache pump", () => {
  it("never runs more than maxInFlight loads and starts the next as one finishes", async () => {
    const t = setup({ maxInFlight: 2 });
    t.wantNow("a", "b", "c", "d");
    expect(t.started).toEqual(["a", "b"]);
    expect(t.cache.stats().inFlight).toBe(2);
    expect(t.cache.get("c")).toBeUndefined();

    await t.settle("a", t.ready("a", 1));
    expect(t.started).toEqual(["a", "b", "c"]);
    expect(t.cache.stats().inFlight).toBe(2);

    await t.settle("b", t.ready("b", 1));
    expect(t.started).toEqual(["a", "b", "c", "d"]);
    expect(t.cache.stats().inFlight).toBe(2);

    await t.settle("c", t.ready("c", 1));
    await t.settle("d", t.ready("d", 1));
    expect(t.cache.stats().inFlight).toBe(0);
  });

  it("loads in wanted order and does not reload cached or loading tiles", async () => {
    const t = setup();
    t.wantNow("b", "a");
    expect(t.started).toEqual(["b", "a"]);
    await t.settle("b", t.ready("b", 1));
    t.wantNow("b", "a", "c");
    expect(t.started).toEqual(["b", "a", "c"]);
  });

  it("notifies subscribers after each load until they unsubscribe", async () => {
    const t = setup();
    const listener = jest.fn();
    const unsubscribe = t.cache.subscribe(listener);
    t.wantNow("a", "b");
    await t.settle("a", t.ready("a", 1));
    expect(listener).toHaveBeenCalledTimes(1);

    unsubscribe();
    await t.settle("b", t.ready("b", 1));
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe("tile cache retry", () => {
  it("retries a failed tile only after the retry window has passed", async () => {
    const t = setup();
    t.wantNow("a");
    await t.fail("a");
    expect(t.cache.get("a")).toEqual({ state: "error", at: 1_000 });
    expect(t.started).toEqual(["a"]);

    t.advance(30_000); // exactly the window: not yet
    t.wantNow("a");
    expect(t.started).toEqual(["a"]);
    expect(t.cache.get("a")?.state).toBe("error");

    t.advance(1);
    t.wantNow("a");
    expect(t.started).toEqual(["a", "a"]);
    expect(t.cache.get("a")?.state).toBe("loading");

    await t.settle("a", t.ready("a"));
    expect(t.cache.get("a")?.state).toBe("ready");
  });

  it("treats a failed response and a rejected load the same", async () => {
    const t = setup();
    t.wantNow("a", "b");
    await t.settle("a", { kind: "error" });
    await t.fail("b");
    expect(t.cache.get("a")).toEqual({ state: "error", at: 1_000 });
    expect(t.cache.get("b")).toEqual({ state: "error", at: 1_000 });
  });

  it("does not retry an empty tile", async () => {
    const t = setup();
    t.wantNow("a");
    await t.settle("a", { kind: "empty" });
    t.advance(60_000);
    t.wantNow("a");
    expect(t.started).toEqual(["a"]);
    expect(t.cache.get("a")?.state).toBe("empty");
  });
});
