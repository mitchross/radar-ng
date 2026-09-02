import { getString, setString } from "../../src/lib/storage";
import {
  cacheManifestIfChanged,
  MANIFEST_CACHE_KEY,
  readCachedManifest,
} from "../../src/lib/manifestCache";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn(),
  setString: jest.fn(),
}));

const mockedGetString = jest.mocked(getString);
const mockedSetString = jest.mocked(setString);

const manifest = {
  schema_version: 2,
  layers: { radar: { timestamps: ["2026-09-02T12:00:00Z"] } },
  tile_url_template: "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
  updated_at: "2026-09-02T12:01:00Z",
};

beforeEach(() => {
  jest.clearAllMocks();
  mockedGetString.mockReturnValue("");
});

describe("manifest cache", () => {
  it("ignores corrupt and runtime-invalid cached manifests", () => {
    mockedGetString.mockReturnValueOnce("not-json");
    expect(readCachedManifest("https://radar.example")).toBeUndefined();

    mockedGetString.mockReturnValueOnce(JSON.stringify({
      serverUrl: "https://radar.example",
      manifest: { ...manifest, layers: { radar: { timestamps: "bad" } } },
    }));
    expect(readCachedManifest("https://radar.example")).toBeUndefined();
  });

  it("does not expose another server's cached manifest", () => {
    mockedGetString.mockReturnValueOnce(JSON.stringify({ serverUrl: "https://one.example", manifest }));
    expect(readCachedManifest("https://two.example")).toBeUndefined();
  });

  it("skips MMKV writes when server and updated_at are unchanged", () => {
    mockedGetString.mockReturnValueOnce(JSON.stringify({
      serverUrl: "https://radar.example",
      manifest: { ...manifest, layers: {} },
    }));

    expect(cacheManifestIfChanged("https://radar.example", manifest)).toBe(false);
    expect(mockedSetString).not.toHaveBeenCalled();
  });

  it("writes a new valid revision and replaces an invalid cache", () => {
    expect(cacheManifestIfChanged("https://radar.example", manifest)).toBe(true);
    expect(mockedSetString).toHaveBeenCalledWith(
      MANIFEST_CACHE_KEY,
      JSON.stringify({ serverUrl: "https://radar.example", manifest }),
    );
  });
});
