import { describe, expect, it } from "vitest";
import { bootLocationStep, cardAfterGeoError, LOCATION_CARD_COPY, locateClickStep, queryLocationPermission } from "./location";

const nav = (query: () => Promise<{ state: PermissionState }>) => ({ permissions: { query } }) as unknown as Navigator;

describe("queryLocationPermission", () => {
  it("returns the Permissions API state", async () => {
    expect(await queryLocationPermission(nav(async () => ({ state: "granted" })))).toBe("granted");
    expect(await queryLocationPermission(nav(async () => ({ state: "denied" })))).toBe("denied");
  });

  it("falls back to unknown without the API or when it throws", async () => {
    expect(await queryLocationPermission({} as Navigator)).toBe("unknown");
    expect(await queryLocationPermission(undefined)).toBe("unknown");
    expect(await queryLocationPermission(nav(() => Promise.reject(new TypeError("unsupported"))))).toBe("unknown");
  });
});

describe("bootLocationStep", () => {
  it("asks first on a first visit when the permission is unasked or unknown", () => {
    expect(bootLocationStep("prompt", false)).toBe("card");
    expect(bootLocationStep("unknown", false)).toBe("card");
  });

  it("skips the card when the permission is settled or a place is saved", () => {
    expect(bootLocationStep("granted", false)).toBe("locate");
    expect(bootLocationStep("denied", false)).toBe("none");
    expect(bootLocationStep("prompt", true)).toBe("none");
    expect(bootLocationStep("granted", true)).toBe("none");
  });
});

describe("locateClickStep", () => {
  it("locates only when granted", () => {
    expect(locateClickStep("granted")).toBe("locate");
    expect(locateClickStep("prompt")).toBe("undetermined");
    expect(locateClickStep("unknown")).toBe("undetermined");
    expect(locateClickStep("denied")).toBe("denied");
  });
});

describe("cardAfterGeoError", () => {
  it("shows the recovery card only for a denial", () => {
    expect(cardAfterGeoError(1)).toBe("denied");
    expect(cardAfterGeoError(2)).toBeNull();
    expect(cardAfterGeoError(3)).toBeNull();
  });
});

describe("LOCATION_CARD_COPY", () => {
  it("names the fallback city and offers Allow only before a denial", () => {
    expect(LOCATION_CARD_COPY.undetermined.body("Grand Rapids")).toContain("Until then this is Grand Rapids.");
    expect(LOCATION_CARD_COPY.undetermined.allow).toBe(true);
    expect(LOCATION_CARD_COPY.denied.allow).toBe(false);
  });
});
