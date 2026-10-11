import { act, fireEvent, render, screen } from "@testing-library/react-native";
import { TimelineBar } from "../../src/components/timeline/TimelineBar";
import { useWeatherStore } from "../../src/stores/useWeatherStore";

const now = Math.floor(Date.now() / 1000);
const frames = Array.from({ length: 13 }, (_, i) => {
  const time = now - (12 - i) * 5 * 60;
  const timestamp = new Date(time * 1000).toISOString();
  return { time, timestamp, path: timestamp, source: "radar" as const };
});

beforeEach(() => {
  useWeatherStore.setState({ frames, currentFrameIndex: 12, isPlaying: false, activeLayer: "radar" });
});

test("the minute clock and identical manifest polls do not re-arm the playback timer", async () => {
  jest.useFakeTimers();
  const setIntervalSpy = jest.spyOn(global, "setInterval");
  try {
    useWeatherStore.setState({ isPlaying: true, playbackSpeed: 2 });
    await render(<TimelineBar />);
    const armed = () => setIntervalSpy.mock.calls.filter(([, ms]) => ms === 500).length;
    expect(armed()).toBe(1);

    await act(async () => { jest.advanceTimersByTime(500); });
    const advanced = useWeatherStore.getState().currentFrameIndex;
    expect(advanced).not.toBe(12);

    // Same frames from a fresh poll, then the 60 s "now" tick.
    await act(async () => {
      useWeatherStore.getState().setFrameTimeline(frames.map((f) => ({ ...f })), 12);
      jest.advanceTimersByTime(60_000);
    });
    expect(armed()).toBe(1);
    expect(useWeatherStore.getState().currentFrameIndex).not.toBe(advanced);
  } finally {
    setIntervalSpy.mockRestore();
    jest.useRealTimers();
  }
});

test("the play control is a labelled button that toggles playback", async () => {
  await render(<TimelineBar />);
  const play = screen.getByRole("button", { name: "Play radar animation" });
  await fireEvent.press(play);
  expect(useWeatherStore.getState().isPlaying).toBe(true);
});
