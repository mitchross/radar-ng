import { fireEvent, screen, waitFor } from "@testing-library/react-native";
import { Text } from "react-native";
import { RadarStatusChip } from "../../src/components/map/RadarStatusChip";
import { RadarStatusNotice, radarNoticeCopy } from "../../src/components/map/RadarStatusNotice";
import { useRadarStatus } from "../../src/hooks/useRadarStatus";
import { radarStatus } from "../../src/lib/radarStatus";
import { storage } from "../../src/lib/storage";
import { renderWithProviders } from "./helpers";

const mockFetchManifest = jest.fn();
jest.mock("../../src/lib/api", () => ({
  ...jest.requireActual("../../src/lib/api"),
  fetchSelfHostedManifest: (...args: unknown[]) => mockFetchManifest(...args),
}));

const NOW = Date.now();
const stale = radarStatus({ frameTimeSeconds: Math.floor(NOW / 1000) - 12 * 60, refreshFailed: false, loading: false, nowMilliseconds: NOW });
const live = radarStatus({ frameTimeSeconds: Math.floor(NOW / 1000) - 60, refreshFailed: false, loading: false, nowMilliseconds: NOW });
const loading = radarStatus({ frameTimeSeconds: null, refreshFailed: false, loading: true });
const unavailable = radarStatus({ frameTimeSeconds: null, refreshFailed: true, loading: false });
const offline = radarStatus({ frameTimeSeconds: Math.floor(NOW / 1000) - 60, refreshFailed: true, loading: false });

beforeEach(() => {
  mockFetchManifest.mockReset();
  // The manifest disk cache would otherwise seed the next test's query.
  storage.clearAll();
});

describe("radar status chip", () => {
  test("shows the same freshness label the Home mini-map uses, announced as an alert when not live", async () => {
    await renderWithProviders(<RadarStatusChip status={stale} />);
    expect(screen.getByText("UPDATED 12M AGO")).toBeOnTheScreen();
    expect(screen.getByRole("alert", { name: "Radar updated 12 minutes ago" })).toBeOnTheScreen();
  });

  test("a live chip is plain, not an alert", async () => {
    await renderWithProviders(<RadarStatusChip status={live} />);
    expect(screen.getByText("LIVE")).toBeOnTheScreen();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("radar status notice", () => {
  test("says nothing for live or stale radar", () => {
    expect(radarNoticeCopy(live, true)).toBeNull();
    expect(radarNoticeCopy(stale, true)).toBeNull();
  });

  test("shows a loading card before the first manifest", async () => {
    await renderWithProviders(<RadarStatusNotice status={loading} hasFrames={false} onRetry={jest.fn()} top={0} />);
    expect(screen.getByText("Loading radar")).toBeOnTheScreen();
    expect(screen.queryByRole("button")).toBeNull();
  });

  test("offers retry when the server is unreachable", async () => {
    const onRetry = jest.fn();
    await renderWithProviders(<RadarStatusNotice status={unavailable} hasFrames={false} onRetry={onRetry} top={0} />);
    expect(screen.getByRole("alert", { name: /Radar unavailable/ })).toBeOnTheScreen();
    await fireEvent.press(screen.getByRole("button", { name: "Retry loading radar" }));
    expect(onRetry).toHaveBeenCalled();
  });

  test("explains that cached frames are shown while offline", async () => {
    await renderWithProviders(<RadarStatusNotice status={offline} hasFrames onRetry={jest.fn()} top={0} />);
    expect(screen.getByText("Radar offline")).toBeOnTheScreen();
    expect(screen.getByText(/last saved observation/)).toBeOnTheScreen();
  });
});

function StatusProbe() {
  const { status } = useRadarStatus();
  return <Text testID="status">{status.label}</Text>;
}

describe("useRadarStatus", () => {
  test("reports LOADING before the first manifest, then LIVE from the latest observed frame", async () => {
    let resolve: (value: unknown) => void = () => {};
    mockFetchManifest.mockReturnValue(new Promise((r) => { resolve = r; }));
    await renderWithProviders(<StatusProbe />);
    expect(screen.getByTestId("status")).toHaveTextContent("LOADING");

    const timestamp = new Date(NOW - 2 * 60_000).toISOString();
    resolve({
      layers: { radar: { timestamps: [timestamp], frames: [{ timestamp, path: timestamp }] } },
      tile_url_template: "https://example.test/{layer}/{timestamp}/{z}/{x}/{y}.png",
      updated_at: new Date(NOW).toISOString(),
    });
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("LIVE"));
  });

  test("reports UNAVAILABLE when the server fails with nothing cached", async () => {
    mockFetchManifest.mockRejectedValue(new Error("Tile server error: 503"));
    await renderWithProviders(<StatusProbe />);
    await waitFor(() => expect(screen.getByTestId("status")).toHaveTextContent("UNAVAILABLE"));
  });
});
