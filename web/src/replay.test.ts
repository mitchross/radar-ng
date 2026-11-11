import { afterEach, describe, expect, it, vi } from "vitest";
import { bannerHtml, caseSpanLabel, fetchReplayCatalogue, fetchReplayManifest, fetchReplayWarnings, pickerHtml } from "./replay";
import type { ReplayCase } from "./shared/replayApi";

const c: ReplayCase = {
  id: "rolling-fork-2023", title: "Rolling Fork <MS>", summary: "EF4 at night", start: "2023-03-24T23:00:00Z", end: "2023-03-25T03:00:00Z",
  timezone: "America/Chicago", bounds: [-92, 31, -88, 35], revision: "r1", layers: ["radar"],
  manifest_url: "/api/replays/rolling-fork-2023/r1/manifest.json", warnings_url: "/api/replays/rolling-fork-2023/r1/warnings.geojson", sources: [{ name: "NOAA MRMS", base_url: "https://x" }],
};
const reply = (body: unknown, ok = true) => vi.stubGlobal("fetch", vi.fn(async () => ({ ok, status: ok ? 200 : 404, json: async () => body })));
afterEach(() => vi.unstubAllGlobals());

describe("replay client", () => {
  it("reads the catalogue from our server and rejects drift", async () => {
    reply({ schema_version: 1, cases: [c] });
    expect((await fetchReplayCatalogue()).cases).toHaveLength(1);
    expect(fetch).toHaveBeenCalledWith("/api/replays", { signal: undefined });
    reply({ schema_version: 2, cases: [] });
    await expect(fetchReplayCatalogue()).rejects.toThrow(/contract/);
  });
  it("errors, never falls back, on a missing case", async () => {
    reply({}, false);
    await expect(fetchReplayManifest(c)).rejects.toThrow(/404/);
    reply({ layers: {} });
    await expect(fetchReplayManifest(c)).rejects.toThrow(/base radar/);
    reply({ type: "nope" });
    await expect(fetchReplayWarnings(c)).rejects.toThrow(/contract/);
  });
  it("uses the catalogue's exact URLs", async () => {
    reply({ schema_version: 2, tile_url_template: "/replays/x/r1/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png", layers: { radar: { frames: [] } } });
    await fetchReplayManifest(c);
    expect(fetch).toHaveBeenCalledWith(c.manifest_url, { signal: undefined });
  });
});

describe("markup", () => {
  it("labels the span in the case zone and escapes text", () => {
    expect(caseSpanLabel(c)).toBe("Fri, Mar 24, 2023, 6:00 PM CDT to 10:00 PM CDT");
    const html = pickerHtml([c]);
    expect(html).toContain("Rolling Fork &lt;MS&gt;");
    expect(html).toContain('data-revision="r1"');
    expect(pickerHtml([])).toMatch(/No replays/);
  });
  it("always names historical replay and offers return to live", () => {
    const html = bannerHtml(c, "Couldn't load");
    expect(html).toContain("Historical replay");
    expect(html).toContain("Return to live");
    expect(html).toContain("Couldn&#39;t load");
  });
});

describe("notification isolation", () => {
  it("keeps replay out of every notification module", async () => {
    const { readFileSync } = await import("node:fs");
    for (const f of ["alertNotifications.ts", "rainAlerts.ts", "stormNotice.ts"]) {
      expect(readFileSync(new URL(`./${f}`, import.meta.url), "utf8")).not.toMatch(/replay/i);
    }
  });
});
