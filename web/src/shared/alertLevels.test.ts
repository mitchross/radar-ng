import { describe, expect, it } from "vitest";
import { alertLevel, selectAlertNotifications, polygonAlertBounds, type AlertFeature } from "./alertLevels";

const now = Date.parse("2026-06-01T12:00:00Z");
function alert(id: string, event = "Flood Warning", extra: Partial<AlertFeature["properties"]> = {}): AlertFeature {
  return { id, properties: { event, severity: "Moderate", expires: "2026-06-02T12:00:00Z", ...extra } };
}

describe("NWS notification levels", () => {
  it("maps critical events and the CAP destructive thunderstorm parameter", () => {
    expect(alertLevel(alert("1", "Tornado Warning").properties)).toBe("critical");
    expect(alertLevel(alert("1", "Flash Flood Emergency").properties)).toBe("critical");
    expect(alertLevel(alert("1", "Flash Flood Warning", { headline: "FLASH FLOOD EMERGENCY for the city" }).properties)).toBe("critical");
    expect(alertLevel(alert("1", "Severe Thunderstorm Warning", { parameters: { thunderstormDamageThreat: ["DESTRUCTIVE"] } }).properties)).toBe("critical");
    expect(alertLevel(alert("1", "Severe Thunderstorm Warning", { parameters: { thunderstormDamageThreat: ["CONSIDERABLE"] } }).properties)).toBe("warning");
  });
  it("keeps watches and advisories as chimes even with Severe CAP severity", () => {
    expect(alertLevel({ event: "Tornado Watch", severity: "Severe" })).toBe("chime");
    expect(alertLevel({ event: "Flood Advisory", severity: "Moderate" })).toBe("chime");
    expect(alertLevel({ event: "Unknown hazard", severity: "Extreme" })).toBe("warning");
  });
});

it("defaults to warnings, optionally adds watches or everything", () => {
  const alerts = [alert("w"), alert("watch", "Tornado Watch"), alert("a", "Flood Advisory")];
  expect(selectAlertNotifications(alerts, {}, "warnings", now).fresh.map(a => a.id)).toEqual(["w"]);
  expect(selectAlertNotifications(alerts, {}, "watches", now).fresh.map(a => a.id)).toEqual(["w", "watch"]);
  expect(selectAlertNotifications(alerts, {}, "everything", now).fresh).toHaveLength(3);
});

it("caps a check at five, most severe first, and silently records the overflow", () => {
  const alerts = [...Array.from({ length: 7 }, (_, i) => alert(String(i))), alert("tor", "Tornado Warning")];
  const first = selectAlertNotifications(alerts, {}, "everything", now);
  expect(first.fresh).toHaveLength(5);
  expect(first.fresh[0].id).toBe("tor");
  expect(selectAlertNotifications(alerts, first.seen, "everything", now).fresh).toEqual([]);
});

it("records reference-linked updates without another notification, including their new IDs", () => {
  const first = selectAlertNotifications([alert("original")], {}, "warnings", now);
  const update = alert("updated", "Flood Warning", { references: [{ identifier: "original" }] });
  const second = selectAlertNotifications([update], first.seen, "warnings", now);
  expect(second.fresh).toEqual([]);
  expect(selectAlertNotifications([alert("updated")], second.seen, "warnings", now).fresh).toEqual([]);
});

it("deduplicates VTEC actions but not a new event number or year", () => {
  const vtec = (action: string, number = "0042", start = "260601T1200Z") => ({ VTEC: [`/O.${action}.KGRR.SV.W.${number}.${start}-260601T1300Z/`] });
  const first = selectAlertNotifications([alert("one", undefined, { parameters: vtec("NEW") })], {}, "warnings", now);
  expect(selectAlertNotifications([alert("two", undefined, { parameters: vtec("CON") })], first.seen, "warnings", now).fresh).toEqual([]);
  expect(selectAlertNotifications([alert("three", undefined, { parameters: vtec("NEW", "0043") })], first.seen, "warnings", now).fresh).toHaveLength(1);
  expect(selectAlertNotifications([alert("four", undefined, { parameters: vtec("NEW", "0042", "270601T1200Z") })], first.seen, "warnings", now).fresh).toHaveLength(1);
});

it("ignores expired, cancelled and non-actual alerts", () => {
  expect(selectAlertNotifications([
    alert("old", undefined, { expires: "2026-05-01T00:00:00Z" }),
    alert("test", undefined, { status: "Test" }),
    alert("cancel", undefined, { messageType: "Cancel" }),
  ], {}, "everything", now).fresh).toEqual([]);
});

it("finds the polygon bounds for notification taps, including multipolygons", () => {
  expect(polygonAlertBounds({ type: "Polygon", coordinates: [[[-86, 42], [-85, 43], [-84, 42], [-86, 42]]] })).toEqual([-86, 42, -84, 43]);
  expect(polygonAlertBounds({ type: "MultiPolygon", coordinates: [[[[-86, 42], [-85, 43], [-84, 42], [-86, 42]]]] })).toEqual([-86, 42, -84, 43]);
  expect(polygonAlertBounds(null)).toBeNull();
});
