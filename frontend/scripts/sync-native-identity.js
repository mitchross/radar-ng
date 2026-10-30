#!/usr/bin/env node
// Copies identity.js values into the Swift sources that can't read env at runtime:
// the App Group, the default server and the basemap raster URL used by the widget,
// CarPlay, the Watch and the shared-state module.
//
//   node scripts/sync-native-identity.js          rewrite the Swift files
//   node scripts/sync-native-identity.js --check  exit 1 if any file is out of date
//
// With no env set this is a no-op: the files already hold the upstream values.
// A fork runs it once after setting its env (FORKING.md), commits the result,
// then rebuilds the native app. __tests__/plugins/identity.test.js runs --check.
const fs = require("fs");
const path = require("path");

const ROOT = path.join(__dirname, "..");
const SHARED = "targets/_shared/RadarShared.swift";

/** Each rule rewrites one `static let name = "…"` literal. */
function rules(identity) {
  return [
    { file: SHARED, name: "appGroup", value: identity.appGroup },
    { file: SHARED, name: "defaultServer", value: identity.defaultServerUrl },
    { file: "modules/radar-shared-state/ios/RadarSharedStateModule.swift", name: "appGroup", value: identity.appGroup },
    { file: "targets/watch/WatchAPI.swift", name: "rasterURL", value: identity.basemapRasterUrl },
    { file: "targets/radar-widget/RadarSnapshot.swift", name: "basemap", value: identity.basemapRasterUrl },
  ];
}

// Simulator-only checks that spell out URLs built from the constants above.
const DERIVED = [
  { file: "tests/watch/PresentationChecks.swift", from: { file: SHARED, name: "defaultServer" }, key: "defaultServerUrl" },
  { file: "tests/watch/PresentationChecks.swift", from: { file: "targets/watch/WatchAPI.swift", name: "rasterURL" }, key: "basemapRasterUrl" },
];

const pattern = (name) => new RegExp(`(static let ${name} = ")([^"]*)(")`);

function read(root, file) {
  return fs.readFileSync(path.join(root, file), "utf8");
}

/** Returns the files whose content would change, after writing them unless `check`. */
function sync({ identity = require("../identity"), root = ROOT, check = false } = {}) {
  const files = new Map();
  const get = (file) => (files.has(file) ? files.get(file) : read(root, file));

  // Literal values before the rewrite, so derived files can follow them.
  const current = (from) => {
    const match = read(root, from.file).match(pattern(from.name));
    if (!match) throw new Error(`${from.file}: no \`static let ${from.name}\``);
    return match[2];
  };
  const derived = DERIVED.map((d) => ({ ...d, old: current(d.from) }));

  for (const rule of rules(identity)) {
    const text = get(rule.file);
    if (!pattern(rule.name).test(text)) throw new Error(`${rule.file}: no \`static let ${rule.name}\``);
    files.set(rule.file, text.replace(pattern(rule.name), (_, a, _old, b) => `${a}${rule.value}${b}`));
  }
  for (const d of derived) {
    if (d.old === identity[d.key]) continue;
    files.set(d.file, get(d.file).split(d.old).join(identity[d.key]));
  }

  const changed = [...files].filter(([file, text]) => text !== read(root, file)).map(([file]) => file);
  if (!check) for (const file of changed) fs.writeFileSync(path.join(root, file), files.get(file));
  return changed;
}

module.exports = { sync };

if (require.main === module) {
  const check = process.argv.includes("--check");
  const changed = sync({ check });
  if (check && changed.length) {
    console.error(`Swift identity is out of date with identity.js:\n  ${changed.join("\n  ")}`);
    console.error("Run: node scripts/sync-native-identity.js");
    process.exit(1);
  }
  console.log(changed.length ? `${check ? "Out of date" : "Updated"}: ${changed.join(", ")}` : "Swift identity is up to date.");
}
