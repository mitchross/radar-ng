import { useWeatherStore } from "../../src/stores/useWeatherStore";
import type { SelectedPlace } from "../../src/types/location";
import { setString } from "../../src/lib/storage";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
  getBoolean: jest.fn(() => false),
  setBoolean: jest.fn(),
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
  });

  it("setFrames updates frames", () => {
    const frames = [{ time: 1000, path: "/a" }, { time: 2000, path: "/b" }];
    useWeatherStore.getState().setFrames(frames);
    expect(useWeatherStore.getState().frames).toEqual(frames);
  });

  it("nextFrame wraps around to 0", () => {
    useWeatherStore.setState({
      frames: [{ time: 1, path: "/a" }, { time: 2, path: "/b" }],
      currentFrameIndex: 1,
    });
    useWeatherStore.getState().nextFrame();
    expect(useWeatherStore.getState().currentFrameIndex).toBe(0);
  });

  it("nextFrame does nothing with empty frames", () => {
    useWeatherStore.getState().nextFrame();
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

  it("setActiveLayer changes active layer", () => {
    useWeatherStore.getState().setActiveLayer("temperature");
    expect(useWeatherStore.getState().activeLayer).toBe("temperature");
  });

  it("toggleOverlay adds and removes overlays", () => {
    useWeatherStore.getState().toggleOverlay("wind");
    expect(useWeatherStore.getState().visibleOverlays.has("wind")).toBe(true);
    useWeatherStore.getState().toggleOverlay("wind");
    expect(useWeatherStore.getState().visibleOverlays.has("wind")).toBe(false);
  });
});
