import { fireEvent, render, screen } from "@testing-library/react-native";
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

test("the play control is a labelled button that toggles playback", async () => {
  await render(<TimelineBar />);
  const play = screen.getByRole("button", { name: "Play radar animation" });
  await fireEvent.press(play);
  expect(useWeatherStore.getState().isPlaying).toBe(true);
});
