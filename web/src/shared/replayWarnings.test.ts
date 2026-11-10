import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { replayWarningsAt, type ReplayWarningHistory } from "./replayWarnings";

const fixture: { history: ReplayWarningHistory; cases: { time: string; expected: string[] }[] } =
  JSON.parse(readFileSync(new URL("./replayWarnings.fixture.json", import.meta.url), "utf8"));

describe("historical warnings", () => {
  for (const { time, expected } of fixture.cases) {
    it(`selects contemporaneous versions at ${time}`, () => {
      expect(replayWarningsAt(fixture.history, Date.parse(time)).map(({ properties: p }) =>
        `${p.eventid}/${p.status}/${p.is_emergency}/${p.is_pds}`)).toEqual(expected);
    });
  }
  it("rejects an invalid playhead", () => {
    expect(replayWarningsAt(fixture.history, NaN)).toEqual([]);
  });
});
