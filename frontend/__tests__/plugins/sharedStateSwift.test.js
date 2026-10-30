const fs = require("fs");
const path = require("path");
const { appGroup } = require("../../identity");

const targets = path.join(__dirname, "../../targets");
const shared = path.join(targets, "_shared/RadarShared.swift");

// apple-targets links targets/_shared into the app and every target, so there is
// one file. A second copy in a target would be a duplicate type in that target.
test("RadarShared.swift exists once, in targets/_shared", () => {
  expect(fs.existsSync(shared)).toBe(true);
  for (const dir of fs.readdirSync(targets).filter((d) => d !== "_shared")) {
    expect(fs.existsSync(path.join(targets, dir, "RadarShared.swift"))).toBe(false);
  }
});

test("Swift and the app agree on the App Group, key and payload fields", () => {
  const swift = fs.readFileSync(shared, "utf8");
  const module = fs.readFileSync(
    path.join(__dirname, "../../modules/radar-shared-state/ios/RadarSharedStateModule.swift"),
    "utf8",
  );
  const ts = fs.readFileSync(path.join(__dirname, "../../src/lib/sharedState.ts"), "utf8");
  for (const needle of [`"${appGroup}"`, '"radarng.shared"']) {
    expect(swift).toContain(needle);
    expect(module).toContain(needle);
  }
  for (const field of ["serverUrl", "palette", "location", "mode", "lat", "lon", "label"]) {
    expect(swift).toContain(`let ${field}`);
    expect(ts).toContain(`${field}:`);
  }
  const app = JSON.parse(fs.readFileSync(path.join(__dirname, "../../app.json"), "utf8"));
  const resolved = require("../../app.config")({ config: app.expo });
  expect(resolved.ios.entitlements["com.apple.security.application-groups"]).toEqual([appGroup]);
  expect(require("../../targets/radar-widget/expo-target.config.js").entitlements).toEqual({
    "com.apple.security.application-groups": [appGroup],
  });
});
