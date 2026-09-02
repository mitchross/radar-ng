import {
  ManifestValidationError,
  parseSelfHostedManifest,
  tryParseSelfHostedManifest,
} from "../../src/lib/manifest";

function validManifest() {
  return {
    schema_version: 2,
    layers: {
      radar: {
        timestamps: ["2026-09-02T12:00:00+00:00"],
        frames: [
          {
            timestamp: "2026-09-02T12:00:00+00:00",
            path: "runs/20260902_12/2026-09-02T12:00:00+00:00",
            kind: "observation",
            issued_at: "2026-09-02T12:00:00+00:00",
            lead_minutes: 0,
            spatial_resolution_km: 1,
            max_zoom: 7,
            palettes: ["classic"],
          },
        ],
        latest: "2026-09-02T12:00:00+00:00",
      },
    },
    tile_url_template: "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
    updated_at: "2026-09-02T12:01:00+00:00",
  };
}

describe("parseSelfHostedManifest", () => {
  it("accepts the current frame contract and legacy timestamp-only layers", () => {
    expect(parseSelfHostedManifest(validManifest())).toEqual(validManifest());
    expect(
      parseSelfHostedManifest({
        layers: { radar: { timestamps: ["2026-09-02T12:00:00Z"] } },
        tile_url_template: "/tiles/{layer}/{timestamp}/{z}/{x}/{y}.png",
        updated_at: "2026-09-02T12:01:00Z",
      }),
    ).toBeDefined();
  });

  it.each([
    ["missing layers", { tile_url_template: "/tiles", updated_at: "2026-09-02T12:01:00Z" }],
    [
      "invalid timestamps",
      { ...validManifest(), layers: { radar: { timestamps: ["yesterday"] } } },
    ],
    [
      "Date.parse-compatible non-ISO timestamps",
      { ...validManifest(), layers: { radar: { timestamps: ["1"] } } },
    ],
    [
      "rolled-over calendar dates",
      { ...validManifest(), layers: { radar: { timestamps: ["2026-02-30T12:00:00Z"] } } },
    ],
    [
      "unsafe frame paths",
      {
        ...validManifest(),
        layers: {
          radar: {
            timestamps: ["2026-09-02T12:00:00+00:00"],
            frames: [{ timestamp: "2026-09-02T12:00:00+00:00", path: "../secret" }],
          },
        },
      },
    ],
    [
      "mismatched frame indexes",
      {
        ...validManifest(),
        layers: {
          radar: {
            timestamps: ["2026-09-02T12:00:00Z"],
            frames: [{ timestamp: "2026-09-02T12:05:00Z", path: "2026-09-02T12:05:00Z" }],
          },
        },
      },
    ],
  ])("rejects %s", (_label, value) => {
    expect(() => parseSelfHostedManifest(value)).toThrow(ManifestValidationError);
    expect(tryParseSelfHostedManifest(value)).toBeUndefined();
  });
});
