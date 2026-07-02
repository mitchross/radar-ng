import { readFileSync } from "fs";
import path from "path";

describe("manifest query contracts", () => {
  const source = readFileSync(
    path.join(__dirname, "../../src/hooks/useManifest.ts"),
    "utf8",
  );

  it("persists the last good manifest to MMKV and hydrates from it", () => {
    expect(source).toContain("setString(storageKey, JSON.stringify(manifest))");
    expect(source).toContain("initialData:");
    // Cached copy must be marked stale so a live refetch still happens.
    expect(source).toContain("initialDataUpdatedAt: 0");
  });

  it("uses one shared query key (no per-consumer duplicates)", () => {
    expect(source).toContain('queryKey: ["manifest", serverUrl]');
    expect(source).not.toContain('"mini"');
  });

  it("RadarMiniMap consumes the shared query instead of polling on its own", () => {
    const mini = readFileSync(
      path.join(__dirname, "../../src/components/home/RadarMiniMap.tsx"),
      "utf8",
    );
    expect(mini).toContain("useManifestQuery(serverUrl)");
    expect(mini).not.toContain("refetchInterval");
    expect(mini).not.toContain("fetchSelfHostedManifest");
  });
});
