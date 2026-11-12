import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  Level2Session, beamHeightM, closeUpFrames, demandTiles, fallbackCopy, groundDistanceM, inCoverage, isFreshSweep, isOwnLevel2Path,
  level2Opacity, level2TileUrl, parseEnvelope, parseSites, parseTimeMs, parseViewStatus, pickNearest, pixelSource, provenance,
  selectStation, underlayIndex, viewCovers, viewProblem, type Level2Site, type Viewport,
} from "./level2";

const T0 = Date.parse("2026-11-12T15:00:00Z");
const iso = (ms: number) => new Date(ms).toISOString();
const VIEW_ID = "a".repeat(32);
const REV = "b".repeat(16);

const site = (id: string, lat: number, lon: number, endMs: number | null, state = "healthy"): Level2Site => ({
  id, name: id, lat, lon, enabled: true, health: { state, eligible: state === "healthy", low_sweep_end: endMs === null ? null : iso(endMs) },
});

function frameJson(endMs: number, i: number) {
  return {
    timestamp: iso(endMs), scan_end: iso(endMs), scan_start: iso(endMs - 30_000), path: `${"c".repeat(60)}${i}/v1-2`, station_id: "KGRR", station_name: "Grand Rapids",
    station_lat: 42.89, station_lon: -85.54, elevation_deg: 0.5, antenna_altitude_m_msl: 266, qc_profile: "level2-conservative-v1", decoder: "nexrad-data 1.0.0",
    dual_pol: { rho: "available", zdr: "absent" }, sources: [{}, {}], sampler_version: "s1", missing_sectors_deg: [], sweep_id: `s${i}`, volume_id: "v#1",
  };
}

function envelope(ends: number[], over: Record<string, unknown> = {}, zoom = 9, tiles: [number, number][] = [[138, 94], [139, 94]]) {
  return {
    schema_version: 1, view_id: VIEW_ID, revision: REV, station: { id: "KGRR", name: "Grand Rapids", lat: 42.89, lon: -85.54 },
    source_zoom: zoom, palettes: [(over.palette as string) ?? "classic"], tile_url_template: "/level2/tiles/{station}/{path}/{palette}/{z}/{x}/{y}.png",
    required_tiles: tiles.map(([x, y]) => ({ z: zoom, x, y })), expires_at: iso(T0 + 60_000),
    manifest: { layers: { "level2-ref-KGRR": { frames: ends.map(frameJson) } } }, ...over,
  };
}

describe("exact boundaries", () => {
  it("accepts a sweep at exactly 600 s and rejects 600 s plus a nanosecond", () => {
    expect(isFreshSweep(iso(T0 - 600_000), T0)).toBe(true);
    expect(isFreshSweep("2026-11-12T14:50:00.000000001Z", T0)).toBe(true);
    expect(isFreshSweep("2026-11-12T14:49:59.9999995Z", T0)).toBe(false);
    expect(parseTimeMs("2026-11-12T14:49:59.9999995Z")).toBeCloseTo(T0 - 600_000.0005, 3);
  });
  it("rejects future and malformed times", () => {
    expect(isFreshSweep(iso(T0 + 1000), T0)).toBe(false);
    expect(isFreshSweep("yesterday", T0)).toBe(false);
    expect(isFreshSweep(null, T0)).toBe(false);
  });
  it("keeps exactly 150 km inside", () => {
    expect(inCoverage(150_000)).toBe(true);
    expect(inCoverage(150_000.01)).toBe(false);
    expect(pixelSource(10, 150_000, 255)).toBe("level2");
    expect(pixelSource(10, 150_001, 255)).toBe("mrms");
  });
  it("shows MRMS through transparent holes and below z8", () => {
    expect(pixelSource(10, 10_000, 0)).toBe("mrms");
    expect(pixelSource(8, 10_000, 255)).toBe("mrms");
  });
});

describe("cross-fade", () => {
  it("ramps linearly from z8 to z9", () => {
    expect([7, 8, 8.25, 8.5, 9, 12].map(level2Opacity)).toEqual([0, 0, 0.25, 0.5, 1, 1]);
  });
});

describe("beam height", () => {
  it("matches the Py-ART 4/3 model", () => {
    expect(beamHeightM(0, 0.5)).toBeCloseTo(0, 0);
    // 100 km at 0.5 deg is about 1.38 km on the standard atmosphere model.
    expect(beamHeightM(100_000, 0.5)!).toBeGreaterThan(1300);
    expect(beamHeightM(100_000, 0.5)!).toBeLessThan(1500);
    expect(beamHeightM(100_000, 1.5)!).toBeGreaterThan(beamHeightM(100_000, 0.5)!);
  });
});

describe("station choice", () => {
  const centre = { lat: 42.9, lon: -85.5 };
  it("takes the nearest healthy fresh station", () => {
    const sites = [site("KFAR", 43.2, -85.5, T0 - 60_000), site("KNEAR", 42.95, -85.5, T0 - 60_000), site("KOLD", 42.91, -85.5, T0 - 700_000)];
    const pick = selectStation(sites, centre, T0);
    expect(pick).toMatchObject({ ok: true, id: "KNEAR" });
  });
  it("breaks equal distance by station id", () => {
    expect(pickNearest([{ id: "KB", distanceM: 5000 }, { id: "KA", distanceM: 5000 }])?.id).toBe("KA");
  });
  it("judges freshness on the client clock even if the server said healthy", () => {
    const pick = selectStation([site("KGRR", 42.9, -85.5, T0 - 601_000)], centre, T0);
    expect(pick).toEqual({ ok: false, reason: "late", enabled: true });
  });
  it("explains warming, range, clock and off", () => {
    expect(selectStation([site("KGRR", 42.9, -85.5, null, "warming")], centre, T0)).toMatchObject({ reason: "warming" });
    expect(selectStation([site("KGRR", 42.9, -85.5, T0, "future_clock")], centre, T0)).toMatchObject({ reason: "future_clock" });
    expect(selectStation([site("KGRR", 48, -85.5, T0)], centre, T0)).toMatchObject({ reason: "out_of_range" });
    expect(selectStation([{ ...site("KGRR", 42.9, -85.5, T0), enabled: false }], centre, T0)).toMatchObject({ reason: "off", enabled: false });
  });
  it("uses the shared great-circle distance", () => {
    expect(groundDistanceM(42.9, -85.5, 42.9, -85.5)).toBe(0);
    expect(groundDistanceM(0, 0, 0, 1)).toBeCloseTo(111_195, -2);
  });
});

describe("contracts", () => {
  it("parses the sites snapshot and rejects drift", () => {
    expect(parseSites({ schema_version: 1, sites: [{ id: "KGRR", name: "G", lat: 1, lon: 2, enabled: true, health: null }] })).toHaveLength(1);
    expect(() => parseSites({ schema_version: 2, sites: [] })).toThrow();
    expect(() => parseSites({ schema_version: 1, sites: [{ id: "KGRR" }] })).toThrow();
  });
  it("parses view status and demands an own manifest URL", () => {
    expect(parseViewStatus({ schema_version: 1, view_id: VIEW_ID, state: "pending", reason: "warming", poll_after_s: 5 })).toMatchObject({ state: "pending", pollAfterMs: 5000 });
    expect(parseViewStatus({ schema_version: 1, view_id: null, state: "fallback", reason: "saturated", poll_after_s: 15 })).toMatchObject({ reason: "saturated" });
    const ready = { schema_version: 1, view_id: VIEW_ID, state: "ready", expires_at: iso(T0), poll_after_s: 15, manifest_url: `/api/level2/views/${VIEW_ID}/${REV}/manifest.json` };
    expect(parseViewStatus(ready).manifestUrl).toBe(ready.manifest_url);
    expect(() => parseViewStatus({ ...ready, manifest_url: "https://nws.noaa.gov/x.json" })).toThrow();
    expect(() => parseViewStatus({ ...ready, manifest_url: `/api/level2/views/${"d".repeat(32)}/${REV}/manifest.json` })).toThrow();
  });
  it("parses the envelope and orders frames", () => {
    const v = parseEnvelope(envelope([T0 - 20_000, T0 - 80_000, T0 - 50_000]));
    expect(v.frames.map((f) => f.scanEndMs)).toEqual([T0 - 80_000, T0 - 50_000, T0 - 20_000]);
    expect(v.frames[0].time).toBe(Math.floor((T0 - 80_000) / 1000));
  });
  it("rejects bad envelopes", () => {
    expect(() => parseEnvelope(envelope([T0, T0]))).toThrow(/repeats/);
    expect(() => parseEnvelope(envelope([1, 2, 3, 4, 5, 6, 7].map((i) => T0 - i * 1000)))).toThrow();
    expect(() => parseEnvelope(envelope([T0], { tile_url_template: "https://example.com/{z}/{x}/{y}.png" }))).toThrow(/foreign/);
    expect(() => parseEnvelope(envelope([T0], { palettes: ["classic", "muted"] }))).toThrow();
    expect(() => parseEnvelope(envelope([T0], { required_tiles: [{ z: 8, x: 1, y: 1 }] }))).toThrow();
    expect(() => parseEnvelope(envelope([T0], { schema_version: 2 }))).toThrow();
  });
  it("builds tile URLs on our own paths only", () => {
    const v = parseEnvelope(envelope([T0]));
    const url = level2TileUrl(v, v.frames[0]);
    expect(url).toMatch(/^\/level2\/tiles\/KGRR\/[a-z0-9]+\/v1-2\/classic\/\{z\}\/\{x\}\/\{y\}\.png$/);
    expect(isOwnLevel2Path(url)).toBe(true);
    expect(isOwnLevel2Path("https://tgftp.nws.noaa.gov/x")).toBe(false);
    expect(isOwnLevel2Path("/api/manifest.json")).toBe(false);
  });
  it("expires on the client clock regardless of the cache", () => {
    const v = parseEnvelope(envelope([T0 - 590_000]));
    expect(viewProblem(v, T0)).toBeNull();
    expect(viewProblem(v, T0 + 11_000)).toBe("late");
    expect(viewProblem(v, T0 + 60_000)).toBe("expired");
  });
});

describe("demand tiles", () => {
  const vp = (zoom: number, bounds: Viewport["bounds"], centre = { lat: 42.9, lon: -85.5 }): Viewport => ({ centre, zoom, bounds, palette: "classic", active: true });
  it("asks for an in-limit set that contains the centre tile", () => {
    const d = demandTiles(vp(10.4, [-90, 40, -81, 46]));
    expect(d.zoom).toBe(10);
    expect(d.tiles.length).toBeLessThanOrEqual(64);
    const xs = d.tiles.map((t) => t[0]);
    const ys = d.tiles.map((t) => t[1]);
    expect(Math.max(...xs) - Math.min(...xs)).toBeLessThan(8);
    expect(Math.max(...ys) - Math.min(...ys)).toBeLessThan(8);
    const cx = ((-85.5 + 180) / 360) * 1024;
    expect(cx).toBeGreaterThanOrEqual(Math.min(...xs));
    expect(cx).toBeLessThanOrEqual(Math.max(...xs) + 1);
  });
  it("clamps the source zoom to 8..13", () => {
    expect(demandTiles(vp(8.2, [-86, 42.5, -85, 43.3])).zoom).toBe(8);
    expect(demandTiles(vp(16, [-85.52, 42.89, -85.48, 42.91])).zoom).toBe(13);
  });
  it("adds a margin only when the limits allow", () => {
    const small = demandTiles(vp(9, [-85.6, 42.8, -85.4, 43]));
    expect(small.tiles.length).toBeGreaterThan(small.core.length);
    const wide = demandTiles(vp(9, [-90, 40, -80, 46]));
    expect(wide.tiles.length).toBe(wide.core.length);
  });
  it("matches a view only when its promised tiles cover the screen", () => {
    const v = parseEnvelope(envelope([T0]));
    expect(viewCovers(v, 9, [[138, 94]])).toBe(true);
    expect(viewCovers(v, 9, [[140, 94]])).toBe(false);
    expect(viewCovers(v, 10, [[138, 94]])).toBe(false);
  });
});

describe("loop and MRMS alignment", () => {
  const mrms = [{ time: 1000 }, { time: 1300 }, { time: 1900 }];
  it("picks the newest observation at or before the scan", () => {
    expect(underlayIndex(1300, mrms)).toBe(1);
    expect(underlayIndex(1500, mrms)).toBe(1);
    expect(underlayIndex(1899, mrms)).toBe(1);
  });
  it("never uses a future observation and gaps after 10 minutes", () => {
    expect(underlayIndex(900, mrms)).toBe(-1);
    expect(underlayIndex(1900 + 600, mrms)).toBe(2);
    expect(underlayIndex(1900 + 601, mrms)).toBe(-1);
  });
  it("keeps every low sweep in order with no thinning", () => {
    const ends = [0, 1, 2, 3, 4, 5].map((i) => T0 - (5 - i) * 20_000);
    const v = parseEnvelope(envelope(ends));
    const loop = closeUpFrames(v, [{ time: Math.floor(T0 / 1000) - 400 }]);
    expect(loop).toHaveLength(6);
    expect(loop.map((f) => f.time)).toEqual([...loop.map((f) => f.time)].sort((a, b) => a - b));
    expect(loop.every((f) => !f.gap)).toBe(true);
  });
  it("shows an explicit gap when MRMS is missing", () => {
    const v = parseEnvelope(envelope([T0]));
    expect(closeUpFrames(v, [])[0]).toMatchObject({ gap: true, mrmsIndex: -1 });
  });
});

describe("provenance", () => {
  const v = parseEnvelope(envelope([T0 - 20_000]));
  it("describes the selected frame, not the newest", () => {
    const p = provenance(v.frames[0], { lat: 43.4, lon: -85.54 }, "America/Detroit");
    expect(p.headline).toMatch(/^Single-site observed · KGRR · scan .* EST · 0\.5° · beam ~/);
    expect(p.headline).toContain("above radar at centre");
    expect(p.groundRange).toMatch(/^\d+ km \(\d+ mi\) from radar at centre$/);
    expect(p.rows.find((r) => r.label === "Scan")?.value).toContain("UTC");
    expect(p.rows.find((r) => r.label === "Beam height")?.value).toContain("not height above terrain");
    expect(p.headline).not.toMatch(/rain|Now/);
  });
  it("short-names every fallback", () => {
    expect(fallbackCopy("late")).toEqual({ title: "MRMS · Level II unavailable", reason: "newest sweep older than 10 min" });
    expect(fallbackCopy("nonsense").reason).toBe("close-up not available here");
  });
});

describe("session lifecycle", () => {
  let now = T0;
  type Reply = { status: number; json: unknown };
  let routes: Record<string, (body?: string) => Reply>;
  const calls: string[] = [];
  let lastBody = "{}";

  const sitesReply = (): Reply => ({ status: 200, json: { schema_version: 1, sites: [site("KGRR", 42.9, -85.5, now - 30_000)] } });
  const ready = (): Reply => ({ status: 200, json: { schema_version: 1, view_id: VIEW_ID, state: "ready", station_id: "KGRR", expires_at: iso(now + 60_000), poll_after_s: 15, manifest_url: `/api/level2/views/${VIEW_ID}/${REV}/manifest.json` } });

  const make = () =>
    new Level2Session({
      now: () => now,
      request: async (path, init) => {
        calls.push(`${init.method} ${path}`);
        if (init.body) lastBody = init.body;
        const key = path.startsWith("/api/level2/views/") && path.endsWith("manifest.json") ? "manifest" : path.startsWith("/api/level2/views/") ? "status" : `${init.method} ${path}`;
        const fn = routes[key];
        if (!fn) throw new Error(`unexpected ${key}`);
        return fn(init.body);
      },
    });
  const vp = (over: Partial<Viewport> = {}): Viewport => ({ centre: { lat: 42.9, lon: -85.5 }, zoom: 10, bounds: [-85.6, 42.8, -85.4, 43], palette: "classic", active: true, ...over });
  const settle = async (ms = 400) => {
    for (let i = 0; i < 8; i++) await vi.advanceTimersByTimeAsync(ms / 8);
  };

  beforeEach(() => {
    vi.useFakeTimers();
    now = T0;
    calls.length = 0;
    routes = {
      "GET /api/level2/sites": sitesReply,
      "POST /api/level2/views": ready,
      manifest: () => {
        const b = JSON.parse(lastBody);
        return { status: 200, json: envelope([now - 30_000, now - 10_000], { palette: b.palette }, b.zoom, b.tiles) };
      },
      status: ready,
    };
  });
  afterEach(() => vi.useRealTimers());

  it("goes ready with the exact tile set and fades with zoom", async () => {
    const s = make();
    s.update(vp({ zoom: 8.5 }));
    await settle();
    const snap = s.snapshot();
    expect(snap.phase).toBe("ready");
    expect(snap.opacity).toBeCloseTo(0.5);
    expect(snap.view?.frames).toHaveLength(2);
    expect(calls).toContain("POST /api/level2/views");
    s.stop();
  });

  it("makes no request below z8 or off Observed radar", async () => {
    const s = make();
    s.update(vp({ zoom: 7.9 }));
    await settle(1000);
    s.update(vp({ active: false }));
    await settle(1000);
    expect(calls).toEqual([]);
    expect(s.snapshot().view).toBeNull();
    s.stop();
  });

  it("falls back at once when the view expires on the client clock", async () => {
    const s = make();
    s.update(vp());
    await settle();
    expect(s.snapshot().view).not.toBeNull();
    now += 601_000;
    expect(s.snapshot().view).toBeNull();
    expect(s.snapshot().opacity).toBe(0);
    s.stop();
  });

  it("drops a held view the moment the palette changes", async () => {
    const s = make();
    s.update(vp());
    await settle();
    s.update(vp({ palette: "muted" }));
    expect(s.snapshot().view).toBeNull();
    s.stop();
  });

  it("ignores a late response after the view changed", async () => {
    let release!: () => void;
    let lastPalette = "classic";
    const gate = new Promise<void>((r) => (release = r));
    const s = new Level2Session({
      now: () => now,
      request: async (path, init) => {
        if (path === "/api/level2/sites") return sitesReply();
        if (init.method === "POST") {
          const body = JSON.parse(init.body!);
          lastPalette = body.palette;
          if (body.palette === "classic") await gate;
          return ready();
        }
        if (path.endsWith("manifest.json")) return { status: 200, json: envelope([now - 10_000], { palette: lastPalette }, 10, [[0, 0]]) };
        return ready();
      },
    });
    s.update(vp());
    await settle();
    s.update(vp({ palette: "muted" }));
    release();
    await settle(2000);
    expect(s.snapshot().view?.palette ?? "muted").toBe("muted");
    s.stop();
  });

  it("goes to fallback with the server's reason and keeps polling slowly", async () => {
    routes["POST /api/level2/views"] = () => ({ status: 200, json: { schema_version: 1, view_id: null, state: "fallback", reason: "saturated", station_id: "KGRR", poll_after_s: 15 } });
    const s = make();
    s.update(vp());
    await settle();
    const snap = s.snapshot();
    expect(snap.view).toBeNull();
    expect(snap.reason).toBe("saturated");
    expect(snap.unavailable).toBe(true);
    s.stop();
  });

  it("stays off for five minutes after a 404 gate", async () => {
    routes["GET /api/level2/sites"] = () => ({ status: 404, json: { detail: "Not Found" } });
    const s = make();
    s.update(vp());
    await settle();
    s.update(vp({ zoom: 11 }));
    await settle(1000);
    expect(calls.filter((c) => c.includes("sites"))).toHaveLength(1);
    expect(s.snapshot().unavailable).toBe(false);
    s.stop();
  });

  it("reports MRMS unavailable after a promised tile fails", async () => {
    const s = make();
    s.update(vp());
    await settle();
    s.reportTileFailure();
    const snap = s.snapshot();
    expect(snap.view).toBeNull();
    expect(snap.reason).toBe("tile_missing");
    s.stop();
  });

  it("never requests a path outside this server", async () => {
    routes["POST /api/level2/views"] = () => ({ status: 200, json: { schema_version: 1, view_id: VIEW_ID, state: "ready", expires_at: iso(now + 60_000), poll_after_s: 15, manifest_url: "https://evil.example/manifest.json" } });
    const s = make();
    s.update(vp());
    await settle();
    expect(calls.every((c) => !c.includes("evil"))).toBe(true);
    expect(s.snapshot().view).toBeNull();
    s.stop();
  });
});
