const fs = require("fs");
const os = require("os");
const path = require("path");

const ROOT = path.join(__dirname, "../..");
const VARS = [
  "RADAR_BUNDLE_ID",
  "RADAR_APP_GROUP",
  "RADAR_ANDROID_PACKAGE",
  "RADAR_APPLE_TEAM_ID",
  "EXPO_PUBLIC_RADAR_DEFAULT_URL",
  "RADAR_BASEMAP_RASTER_URL",
];
const saved = Object.fromEntries(VARS.map((v) => [v, process.env[v]]));

afterEach(() => {
  for (const v of VARS) {
    if (saved[v] === undefined) delete process.env[v];
    else process.env[v] = saved[v];
  }
});

/** Loads `mod` with exactly `vars` set, so identity.js re-reads env. */
function load(mod, vars = {}) {
  for (const v of VARS) delete process.env[v];
  Object.assign(process.env, vars);
  let result;
  jest.isolateModules(() => {
    result = require(mod);
  });
  return result;
}

const appJson = () => JSON.parse(fs.readFileSync(path.join(ROOT, "app.json"), "utf8")).expo;

// The upstream App Store build depends on these exact values. A change here
// re-identifies the shipped app; it should only ever happen on purpose.
test("with no env the identity is the upstream app's", () => {
  expect(load("../../identity")).toEqual({
    bundleId: "com.vanillax.radar-ng",
    appGroup: "group.com.vanillax.radar-ng",
    androidPackage: "com.vanillax.radarng",
    appleTeamId: "FVU6RGL532",
    defaultServerUrl: "https://radar-ng-api.vanillax.me",
    basemapRasterUrl: "https://maps.vanillax.me/raster",
  });
});

test("with no env the resolved app config carries the upstream identity", () => {
  const configure = load("../../app.config");
  const config = configure({ config: appJson() });
  expect(config.ios.bundleIdentifier).toBe("com.vanillax.radar-ng");
  expect(config.ios.appleTeamId).toBe("FVU6RGL532");
  expect(config.ios.entitlements).toEqual({
    "com.apple.security.application-groups": ["group.com.vanillax.radar-ng"],
  });
  expect(config.android.package).toBe("com.vanillax.radarng");
});

test("env overrides every value, and the App Group follows the bundle ID", () => {
  const identity = load("../../identity", {
    RADAR_BUNDLE_ID: "com.example.radar",
    RADAR_ANDROID_PACKAGE: "com.example.radarapp",
    RADAR_APPLE_TEAM_ID: "ABCDE12345",
    EXPO_PUBLIC_RADAR_DEFAULT_URL: "https://radar.example.com/",
    RADAR_BASEMAP_RASTER_URL: "https://maps.example.com/raster/",
  });
  expect(identity).toEqual({
    bundleId: "com.example.radar",
    appGroup: "group.com.example.radar",
    androidPackage: "com.example.radarapp",
    appleTeamId: "ABCDE12345",
    defaultServerUrl: "https://radar.example.com",
    basemapRasterUrl: "https://maps.example.com/raster",
  });
  expect(load("../../identity", { RADAR_APP_GROUP: "group.example.shared" }).appGroup).toBe("group.example.shared");
});

test("app config, widget target and app use the env identity together", () => {
  const vars = { RADAR_BUNDLE_ID: "com.example.radar", RADAR_APPLE_TEAM_ID: "ABCDE12345" };
  const config = load("../../app.config", vars)({ config: appJson() });
  expect(config.ios.bundleIdentifier).toBe("com.example.radar");
  expect(config.ios.appleTeamId).toBe("ABCDE12345");
  expect(config.ios.entitlements["com.apple.security.application-groups"]).toEqual(["group.com.example.radar"]);
  expect(load("../../targets/radar-widget/expo-target.config.js", vars).entitlements).toEqual({
    "com.apple.security.application-groups": ["group.com.example.radar"],
  });
});

test("app.json no longer hard-codes the identity", () => {
  const expo = appJson();
  expect(expo.ios.bundleIdentifier).toBeUndefined();
  expect(expo.ios.appleTeamId).toBeUndefined();
  expect(expo.ios.entitlements).toBeUndefined();
  expect(expo.android.package).toBeUndefined();
});

test("the app's default stack URL matches identity.js and follows the env", () => {
  expect(load("../../src/lib/constants").SELF_HOSTED.DEFAULT_URL).toBe(load("../../identity").defaultServerUrl);
  const url = "https://radar.example.com";
  expect(load("../../src/lib/constants", { EXPO_PUBLIC_RADAR_DEFAULT_URL: `${url}/` }).SELF_HOSTED.DEFAULT_URL).toBe(url);
});

test("the Swift sources match identity.js (run scripts/sync-native-identity.js if not)", () => {
  const { sync } = load("../../scripts/sync-native-identity");
  expect(sync({ identity: load("../../identity"), check: true })).toEqual([]);
});

test("the sync script rewrites every Swift constant for a fork", () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "radar-identity-"));
  for (const dir of ["targets", "modules", "tests"]) fs.cpSync(path.join(ROOT, dir), path.join(tmp, dir), { recursive: true });
  const fork = load("../../identity", {
    RADAR_BUNDLE_ID: "com.example.radar",
    EXPO_PUBLIC_RADAR_DEFAULT_URL: "https://radar.example.com",
    RADAR_BASEMAP_RASTER_URL: "https://maps.example.com/raster",
  });
  const { sync } = load("../../scripts/sync-native-identity");

  expect(sync({ identity: fork, root: tmp }).length).toBeGreaterThan(0);
  expect(sync({ identity: fork, root: tmp, check: true })).toEqual([]);

  const swift = ["targets", "modules", "tests"].flatMap(function walk(entry) {
    const full = path.join(tmp, entry);
    if (fs.statSync(full).isFile()) return entry.endsWith(".swift") ? [entry] : [];
    return fs.readdirSync(full).flatMap((name) => walk(path.join(entry, name)));
  });
  const leftovers = swift.filter((f) => /vanillax/.test(fs.readFileSync(path.join(tmp, f), "utf8")));
  expect(leftovers).toEqual([]);
  const shared = fs.readFileSync(path.join(tmp, "targets/watch/RadarShared.swift"), "utf8");
  expect(shared).toContain('static let appGroup = "group.com.example.radar"');
  expect(shared).toContain('static let defaultServer = "https://radar.example.com"');
  fs.rmSync(tmp, { recursive: true, force: true });
});
