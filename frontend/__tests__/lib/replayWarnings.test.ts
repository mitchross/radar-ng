import { replayWarningsAt, type ReplayWarningHistory } from "../../src/lib/replayWarnings";

const fixture: { history: ReplayWarningHistory; cases: { time: string; expected: string[] }[] } =
  require("../../../web/src/shared/replayWarnings.fixture.json");

describe("historical warnings shared with web", () => {
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
