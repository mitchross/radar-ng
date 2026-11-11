import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { parseReplayCatalogue, replayTileTemplate } from "./replayApi";

const fixture = JSON.parse(readFileSync(new URL("./replayApi.fixture.json", import.meta.url), "utf8"));

describe("replay API contract", () => {
  it("accepts the shared catalogue", () => {
    expect(parseReplayCatalogue(fixture.catalogue).cases.map((c) => c.id)).toEqual(["rolling-fork-2023"]);
  });
  it("rejects a drifted catalogue", () => {
    const bad = structuredClone(fixture.catalogue);
    bad.cases[0].manifest_url = "/api/manifest.json";
    expect(() => parseReplayCatalogue(bad)).toThrow();
    expect(() => parseReplayCatalogue({ schema_version: 2, cases: [] })).toThrow();
  });
  it("requires the case tile template", () => {
    expect(replayTileTemplate(fixture.manifest)).toContain("/replays/rolling-fork-2023/contract-1/tiles/");
    expect(() => replayTileTemplate({ schema_version: 2, tile_url_template: "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png" })).toThrow();
  });
});
