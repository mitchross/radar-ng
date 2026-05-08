const fs = require("fs");
const path = require("path");

const podspecPath = path.join(
  __dirname,
  "..",
  "node_modules",
  "@bacons",
  "apple-targets",
  "ios",
  "ExtensionStorage.podspec",
);

if (!fs.existsSync(podspecPath)) {
  process.exit(0);
}

const contents = fs.readFileSync(podspecPath, "utf8");
const patched = contents.replace(
  /s\.platform\s*=\s*:ios,\s*['"]15\.1['"]/,
  "s.platform       = :ios, '16.4'",
);

if (patched !== contents) {
  fs.writeFileSync(podspecPath, patched);
}
