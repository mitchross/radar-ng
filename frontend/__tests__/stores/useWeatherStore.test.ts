import { useWeatherStore } from "../../src/stores/useWeatherStore";
import type { SelectedPlace } from "../../src/types/location";
import { setString } from "../../src/lib/storage";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
}));

beforeEach(() => {
  jest.clearAllMocks();
  useWeatherStore.setState(useWeatherStore.getInitialState());
});

describe("useWeatherStore", () => {
  it("starts with default values", () => {
    const state = useWeatherStore.getState();
    expect(state.frames).toEqual([]);
    expect(state.currentFrameIndex).toBe(-1);
    expect(state.isPlaying).toBe(false);
    expect(state.radarOpacity).toBe(0.8);
    expect(state.activeLayer).toBe("radar");
    expect(state.serverUrl).toContain("radar-ng-api");
    expect(state.appearanceMode).toBe("system");
  });

  it("persists an explicit appearance mode independently of map style", () => {
    useWeatherStore.getState().setMapStyle("satellite");
    useWeatherStore.getState().setAppearanceMode("dark");

    expect(useWeatherStore.getState().appearanceMode).toBe("dark");
    expect(useWeatherStore.getState().mapStyle).toBe("satellite");
    expect(setString).toHaveBeenCalledWith("appearanceMode", "dark");
  });

  it("publishes frames and selected index in one coherent store update", () => {
    const frames = [
      { time: 1000, timestamp: "1970-01-01T00:16:40Z", path: "/a" },
      { time: 2000, timestamp: "1970-01-01T00:33:20Z", path: "/b" },
    ];
    const snapshots: Array<{ frameCount: number; currentFrameIndex: number }> = [];
    const unsubscribe = useWeatherStore.subscribe((state) => {
      snapshots.push({ frameCount: state.frames.length, currentFrameIndex: state.currentFrameIndex });
    });

    useWeatherStore.getState().setFrameTimeline(frames, 1);
    unsubscribe();

    expect(useWeatherStore.getState().frames).toEqual(frames);
    expect(useWeatherStore.getState().currentFrameIndex).toBe(1);
    expect(snapshots).toEqual([{ frameCount: 2, currentFrameIndex: 1 }]);
  });

  it("preserves selected wall-clock time when a manifest refresh shifts indices", () => {
    const before = [
      { time: 1000, timestamp: "1970-01-01T00:16:40Z", path: "/a" },
      { time: 2000, timestamp: "1970-01-01T00:33:20Z", path: "/b" },
    ];
    const after = [
      { time: 500, timestamp: "1970-01-01T00:08:20Z", path: "/new" },
      ...before,
    ];
    useWeatherStore.getState().setFrameTimeline(before, 0);
    useWeatherStore.getState().setFrameTimeline(after, 2);

    expect(useWeatherStore.getState().currentFrameIndex).toBe(1);
    expect(useWeatherStore.getState().frames[1].time).toBe(1000);
  });

  it("resets the selected index when a manifest publishes no frames", () => {
    useWeatherStore.getState().setFrameTimeline(
      [{ time: 1000, timestamp: "1970-01-01T00:16:40Z", path: "/a" }],
      0,
    );
    useWeatherStore.getState().setFrameTimeline([], -1);

    expect(useWeatherStore.getState().frames).toEqual([]);
    expect(useWeatherStore.getState().currentFrameIndex).toBe(-1);
  });

  it("togglePlaying flips isPlaying", () => {
    expect(useWeatherStore.getState().isPlaying).toBe(false);
    useWeatherStore.getState().togglePlaying();
    expect(useWeatherStore.getState().isPlaying).toBe(true);
  });

  it("setLocation updates lat/lon", () => {
    useWeatherStore.getState().setLocation(38.9, -77.0);
    const { latitude, longitude } = useWeatherStore.getState();
    expect(latitude).toBe(38.9);
    expect(longitude).toBe(-77.0);
  });

  it("setSelectedPlace switches to city mode and persists the city", () => {
    const place: SelectedPlace = {
      id: 4994358,
      name: "Grand Rapids",
      latitude: 42.9634,
      longitude: -85.6681,
      admin1: "Michigan",
      country: "United States",
      countryCode: "US",
    };

    useWeatherStore.getState().setSelectedPlace(place);

    const state = useWeatherStore.getState();
    expect(state.locationMode).toBe("city");
    expect(state.selectedPlace).toEqual(place);
    expect(state.latitude).toBe(place.latitude);
    expect(state.longitude).toBe(place.longitude);
    expect(setString).toHaveBeenCalledWith("locationMode", "city");
    expect(setString).toHaveBeenCalledWith("selectedPlace", JSON.stringify(place));
  });

  it("useDeviceLocation switches back to device mode without clearing the saved city", () => {
    const place: SelectedPlace = {
      id: 4994358,
      name: "Grand Rapids",
      latitude: 42.9634,
      longitude: -85.6681,
    };

    useWeatherStore.getState().setSelectedPlace(place);
    useWeatherStore.getState().useDeviceLocation();

    const state = useWeatherStore.getState();
    expect(state.locationMode).toBe("device");
    expect(state.selectedPlace).toEqual(place);
    expect(setString).toHaveBeenCalledWith("locationMode", "device");
  });

  it("starts with no playback window and publishes one from the timeline", () => {
    expect(useWeatherStore.getState().playbackWindow).toBeNull();
    useWeatherStore.getState().setPlaybackWindow({ start: 3, end: 9 });
    expect(useWeatherStore.getState().playbackWindow).toEqual({ start: 3, end: 9 });
    useWeatherStore.getState().setPlaybackWindow(null);
    expect(useWeatherStore.getState().playbackWindow).toBeNull();
  });

  it("setPlaybackWindow keeps the same reference for an equal window (no re-render churn)", () => {
    useWeatherStore.getState().setPlaybackWindow({ start: 0, end: 5 });
    const first = useWeatherStore.getState().playbackWindow;
    useWeatherStore.getState().setPlaybackWindow({ start: 0, end: 5 });
    expect(useWeatherStore.getState().playbackWindow).toBe(first);
  });

  it("setActiveLayer changes active layer", () => {
    useWeatherStore.getState().setActiveLayer("temperature");
    expect(useWeatherStore.getState().activeLayer).toBe("temperature");
  });

  it("setServerUrl persists a valid origin and ignores garbage", () => {
    useWeatherStore.getState().setServerUrl("http://192.168.1.10:8080/");
    expect(useWeatherStore.getState().serverUrl).toBe("http://192.168.1.10:8080");
    expect(setString).toHaveBeenCalledWith("serverUrl", "http://192.168.1.10:8080");

    useWeatherStore.getState().setServerUrl("not a url");
    expect(useWeatherStore.getState().serverUrl).toBe("http://192.168.1.10:8080");
  });
});
