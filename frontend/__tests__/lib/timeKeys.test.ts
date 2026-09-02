import { hourKey } from "../../src/lib/timeKeys";

describe("hourKey", () => {
  it("floors an ISO timestamp to the UTC hour", () => {
    expect(hourKey("2026-09-02T14:24:00Z")).toBe("2026-09-02T14:00:00Z");
    expect(hourKey("2026-09-02T14:00:00Z")).toBe("2026-09-02T14:00:00Z");
    expect(hourKey("2026-09-02T14:59:59.999Z")).toBe("2026-09-02T14:00:00Z");
  });

  it("normalises offsets to UTC so one HRRR hour has one key", () => {
    expect(hourKey("2026-09-02T10:30:00-04:00")).toBe("2026-09-02T14:00:00Z");
  });

  it("passes unparseable input through unchanged", () => {
    expect(hourKey("latest")).toBe("latest");
  });
});
