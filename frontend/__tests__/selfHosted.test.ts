import { readdirSync, readFileSync, statSync } from "fs";
import path from "path";

/**
 * Radar NG only ever talks to the user's own servers at runtime: no API keys,
 * no third-party map, imagery, geocoding or tile services (plan Phase S).
 * Public data is fine when the *server* fetches it. These checks fail the build
 * if a client-side dependency on another host creeps back in.
 */

const ROOT = path.join(__dirname, "..");
const SCANNED = ["src", "targets", "plugins", "app.json", "app.config.js", ".env.production"];

// The deployment's own servers, plus placeholders used in docs and examples.
const ALLOWED_HOSTS = new Set([
  "radar-ng-api.vanillax.me",
  "maps.vanillax.me",
  "localhost",
  "127.0.0.1",
  "10.0.2.2",
]);
const ALLOWED_SUFFIXES = [".example.com", ".example"];

function files(entry: string): string[] {
  const full = path.join(ROOT, entry);
  if (!statSync(full, { throwIfNoEntry: false })) return [];
  if (statSync(full).isFile()) return [full];
  return readdirSync(full, { withFileTypes: true }).flatMap((dirent) =>
    dirent.name === "node_modules" ? [] : files(path.join(entry, dirent.name)),
  );
}

const sources = SCANNED.flatMap(files).filter((f) => /\.(tsx?|jsx?|swift|json|plist)$|\.env/.test(f));

describe("self-hosted only", () => {
  it("references no host outside the user's own servers", () => {
    const offenders: string[] = [];
    for (const file of sources) {
      const text = readFileSync(file, "utf8");
      for (const match of text.matchAll(/https?:\/\/([a-zA-Z0-9.-]+)/g)) {
        const host = match[1].toLowerCase();
        // Property-list DOCTYPE lines name Apple's DTD; nothing fetches it.
        if (file.endsWith(".plist") && host === "www.apple.com") continue;
        if (ALLOWED_HOSTS.has(host) || ALLOWED_SUFFIXES.some((s) => host.endsWith(s))) continue;
        offenders.push(`${path.relative(ROOT, file)}: ${match[0]}`);
      }
    }
    expect(offenders).toEqual([]);
  });

  it("uses no platform map or geocoding service outside CarPlay", () => {
    // CarPlay routing (MKDirections/MKLocalSearch) is the documented exception
    // while the entitlement is pending with Apple (plan Phase S, S7).
    const forbidden = /\bMKMapSnapshotter\b|\bCLGeocoder\b|\bMKLocalSearch\b|\breverseGeocodeAsync\b|\bgeocodeAsync\b|^import MapKit$/m;
    const offenders = sources
      .filter((file) => !file.includes(`${path.sep}targets${path.sep}carplay${path.sep}`))
      .filter((file) => forbidden.test(readFileSync(file, "utf8")))
      .map((file) => path.relative(ROOT, file));
    expect(offenders).toEqual([]);
  });
});
