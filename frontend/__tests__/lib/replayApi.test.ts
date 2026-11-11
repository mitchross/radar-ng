import { parseReplayCatalogue, replayTileTemplate } from "../../src/lib/replayApi";

const fixture = require("../../../web/src/shared/replayApi.fixture.json");

describe("replay API contract shared with web", () => {
  it("accepts the shared catalogue", () => {
    expect(parseReplayCatalogue(fixture.catalogue).cases.map((c) => c.id)).toEqual(["rolling-fork-2023"]);
  });
  it("rejects a drifted catalogue", () => {
    const bad = JSON.parse(JSON.stringify(fixture.catalogue));
    bad.cases[0].warnings_url = "/api/alerts";
    expect(() => parseReplayCatalogue(bad)).toThrow();
  });
  it("requires the case tile template", () => {
    expect(replayTileTemplate(fixture.manifest)).toContain("/replays/rolling-fork-2023/contract-1/tiles/");
  });
});
