const { withAppBuildGradle } = require("@expo/config-plugins");

// Release builds are signed locally with the Play upload key (Play App Signing
// re-signs for distribution). The key and passwords never live in the repo:
// scripts/build-android-release.sh loads them from ~/.radar-ng and hands them
// to Gradle as ORG_GRADLE_PROJECT_RADAR_UPLOAD_* environment variables.
// Without them, release falls back to the debug key, fine for local QA only.
const RELEASE_SIGNING = `
        if (findProperty('RADAR_UPLOAD_STORE_FILE')) {
            release {
                storeFile file(findProperty('RADAR_UPLOAD_STORE_FILE'))
                storePassword findProperty('RADAR_UPLOAD_STORE_PASSWORD')
                keyAlias findProperty('RADAR_UPLOAD_KEY_ALIAS')
                keyPassword findProperty('RADAR_UPLOAD_KEY_PASSWORD')
            }
        }`;
const RELEASE_SELECTOR =
  "signingConfig findProperty('RADAR_UPLOAD_STORE_FILE') ? signingConfigs.release : signingConfigs.debug";

function applyReleaseSigning(gradle) {
  if (gradle.includes("RADAR_UPLOAD_STORE_FILE")) return gradle;
  const debugConfig = /(signingConfigs\s*\{\s*debug\s*\{[^}]*\})/;
  if (!debugConfig.test(gradle)) throw new Error("app/build.gradle: signingConfigs.debug not found");
  let out = gradle.replace(debugConfig, `$1${RELEASE_SIGNING}`);
  const releaseType = /(buildTypes\s*\{[\s\S]*?release\s*\{[\s\S]*?)signingConfig signingConfigs\.debug/;
  if (!releaseType.test(out)) throw new Error("app/build.gradle: release signingConfig not found");
  out = out.replace(releaseType, `$1${RELEASE_SELECTOR}`);
  return out;
}

module.exports = (config) =>
  withAppBuildGradle(config, (c) => {
    c.modResults.contents = applyReleaseSigning(c.modResults.contents);
    return c;
  });
module.exports.applyReleaseSigning = applyReleaseSigning;
