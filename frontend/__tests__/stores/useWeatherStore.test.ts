import { useWeatherStore } from "../../src/stores/useWeatherStore";

jest.mock("../../src/lib/storage", () => ({
  getString: jest.fn((_k: string, d: string) => d),
  setString: jest.fn(),
  getBoolean: jest.fn(() => false),
  setBoolean: jest.fn(),
}));

beforeEach(() => {
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
