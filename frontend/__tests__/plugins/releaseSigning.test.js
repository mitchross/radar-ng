const fs = require("fs");
const path = require("path");
const { applyReleaseSigning } = require("../../plugins/withAndroidReleaseSigning");

// Shape of the build.gradle Expo SDK 57 generates (signing section only).
const GENERATED = `
android {
    signingConfigs {
        debug {
            storeFile file('debug.keystore')
            storePassword 'android'
            keyAlias 'androiddebugkey'
            keyPassword 'android'
        }
    }
    buildTypes {
        debug {
            signingConfig signingConfigs.debug
        }
        release {
            // Caution! In production, you need to generate your own keystore file.
            signingConfig signingConfigs.debug
            minifyEnabled enableMinifyInReleaseBuilds
        }
    }
}
`;

test("adds an upload-key release config read from Gradle properties", () => {
  const out = applyReleaseSigning(GENERATED);
  expect(out).toMatch(/if \(findProperty\('RADAR_UPLOAD_STORE_FILE'\)\) \{\s*release \{/);
  expect(out).toContain("keyPassword findProperty('RADAR_UPLOAD_KEY_PASSWORD')");
});

test("release uses the upload key when present; debug builds are untouched", () => {
  const out = applyReleaseSigning(GENERATED);
  const release = out.slice(out.indexOf("        release {\n            // Caution"));
  expect(release).toContain(
    "signingConfig findProperty('RADAR_UPLOAD_STORE_FILE') ? signingConfigs.release : signingConfigs.debug",
  );
  expect(out).toMatch(/debug \{\s*signingConfig signingConfigs\.debug\s*\}/);
});

test("is idempotent and never embeds secrets", () => {
  const once = applyReleaseSigning(GENERATED);
  expect(applyReleaseSigning(once)).toBe(once);
  expect(once).not.toMatch(/storePassword '(?!android')/);
});

test("fails loudly if Expo's template changes shape", () => {
  expect(() => applyReleaseSigning("android { }")).toThrow("signingConfigs.debug");
});

test("matches the build.gradle the current SDK generates", () => {
  const generated = path.join(__dirname, "../../android/app/build.gradle");
  if (!fs.existsSync(generated)) return;
  const contents = fs.readFileSync(generated, "utf8");
  if (contents.includes("RADAR_UPLOAD_STORE_FILE")) return;
  expect(() => applyReleaseSigning(contents)).not.toThrow();
});
