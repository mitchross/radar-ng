import { describeNowcastSkill } from "../../src/lib/nowcastSkill";

describe("describeNowcastSkill", () => {
  it("withholds a claim without enough runs or observed rain", () => {
    expect(describeNowcastSkill(null)).toBeNull();
    expect(describeNowcastSkill(undefined)).toBeNull();
    expect(
      describeNowcastSkill({ lead_minutes: 30, window_hours: 24, runs: 3, pod: 0.9, far: 0.1, csi: 0.8, persistence_csi: 0.5 }),
    ).toBeNull();
    expect(
      describeNowcastSkill({ lead_minutes: 30, window_hours: 24, runs: 30, pod: null, far: null, csi: null, persistence_csi: null }),
    ).toBeNull();
  });

  it("turns the headline into plain words", () => {
    const view = describeNowcastSkill({
      lead_minutes: 30,
      window_hours: 24,
      runs: 40,
      pod: 0.873,
      far: 0.094,
      csi: 0.8,
      persistence_csi: 0.61,
    })!;
    expect(view.hitRatePct).toBe(87);
    expect(view.falseAlarmPct).toBe(9);
    expect(view.beatsPersistence).toBe(true);
    expect(view.sentence).toBe("Caught 87% of rain 30 min out today, 9% false alarms");
    expect(view.short).toBe("87% hit rate today");
  });

  it("copes with no forecast rain and other windows", () => {
    const view = describeNowcastSkill({
      lead_minutes: 15,
      window_hours: 6,
      runs: 10,
      pod: 0.5,
      far: null,
      csi: 0.5,
      persistence_csi: null,
    })!;
    expect(view.falseAlarmPct).toBeNull();
    expect(view.beatsPersistence).toBeNull();
    expect(view.sentence).toBe("Caught 50% of rain 15 min out over 6 h");
  });
});
