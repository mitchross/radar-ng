import * as Notifications from "expo-notifications";
import { checkStormNotice } from "../../src/lib/stormNoticeScheduler";
import type { StormCellInfo } from "../../src/lib/stormTracks";

jest.mock("../../src/lib/storage", () => {
  const data: Record<string, string> = {};
  return {
    getString: (k: string, d: string) => data[k] ?? d,
    setString: (k: string, v: string) => { data[k] = v; },
    __clear: () => { for (const k of Object.keys(data)) delete data[k]; },
  };
});
jest.mock("expo-notifications", () => ({
  scheduleNotificationAsync: jest.fn(async () => "id"),
  setNotificationChannelAsync: jest.fn(async () => undefined),
  AndroidImportance: { HIGH: 4 },
}));

const cell = (over: Partial<StormCellInfo> = {}): StormCellInfo => ({
  id: 1, lon: -85.95, lat: 42.96, peakDbz: 58, areaKm2: 120, speedMph: 37.3, bearing: 90, confidence: 0.9,
  eastKmh: 60, northKmh: 0, history: [[0, -86.4, 42.96], [300, -86.3, 42.96], [600, -86.2, 42.96]], ...over,
});
const place: [number, number] = [-85.67, 42.96];
const schedule = Notifications.scheduleNotificationAsync as jest.Mock;

beforeEach(() => {
  (jest.requireMock("../../src/lib/storage") as { __clear: () => void }).__clear();
  schedule.mockClear();
});

describe("checkStormNotice", () => {
  it("notifies once for an approaching cell, with the estimate wording", async () => {
    await checkStormNotice([cell()], place, 1000);
    expect(schedule).toHaveBeenCalledTimes(1);
    const { content } = schedule.mock.calls[0][0];
    expect(content.title).toMatch(/~\d+ min/);
    expect(content.body).toMatch(/not an NWS warning/);
  });
  it("stays quiet on repeat polls of the same cell", async () => {
    await checkStormNotice([cell()], place, 1000);
    await checkStormNotice([cell()], place, 61_000);
    await checkStormNotice([cell({ lon: -85.9 })], place, 121_000);
    expect(schedule).toHaveBeenCalledTimes(1);
  });
  it("announces a different cell", async () => {
    await checkStormNotice([cell()], place, 1000);
    await checkStormNotice([cell({ id: 2 })], place, 2000);
    expect(schedule).toHaveBeenCalledTimes(2);
  });
  it("ignores a cell passing wide or receding", async () => {
    await checkStormNotice([cell({ lat: 43.3 }), cell({ id: 3, eastKmh: -60, bearing: 270 })], place, 1000);
    expect(schedule).not.toHaveBeenCalled();
  });
});
