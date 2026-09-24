const fs = require("fs");
const path = require("path");

const targets = path.join(__dirname, "../../targets");
const copies = ["radar-widget", "carplay", "watch"].map((t) => path.join(targets, t, "RadarShared.swift"));

test("every native target reads shared state with the same RadarShared.swift", () => {
  const [first, ...rest] = copies.map((p) => fs.readFileSync(p, "utf8"));
  for (const other of rest) expect(other).toBe(first);
});

test("Swift and the app agree on the App Group, key and payload fields", () => {
  const swift = fs.readFileSync(copies[0], "utf8");
  const module = fs.readFileSync(
    path.join(__dirname, "../../modules/radar-shared-state/ios/RadarSharedStateModule.swift"),
    "utf8",
  );
  const ts = fs.readFileSync(path.join(__dirname, "../../src/lib/sharedState.ts"), "utf8");
  for (const needle of ['"group.com.vanillax.radar-ng"', '"radarng.shared"']) {
    expect(swift).toContain(needle);
    expect(module).toContain(needle);
  }
  for (const field of ["serverUrl", "palette", "location", "mode", "lat", "lon", "label"]) {
    expect(swift).toContain(`let ${field}`);
    expect(ts).toContain(`${field}:`);
  }
  const app = JSON.parse(fs.readFileSync(path.join(__dirname, "../../app.json"), "utf8"));
  expect(app.expo.ios.entitlements["com.apple.security.application-groups"]).toEqual(["group.com.vanillax.radar-ng"]);
  expect(require("../../targets/radar-widget/expo-target.config.js").entitlements).toEqual({
    "com.apple.security.application-groups": ["group.com.vanillax.radar-ng"],
  });
});
