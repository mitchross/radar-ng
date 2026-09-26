import { act, fireEvent, screen } from "@testing-library/react-native";
import { StyleSheet } from "react-native";
import { WeatherMap } from "../../src/components/map/WeatherMap";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

const mockCamera = { setStop: jest.fn(), fitBounds: jest.fn() };

jest.mock("@maplibre/maplibre-react-native", () => {
  const { forwardRef, useImperativeHandle } = jest.requireActual<typeof import("react")>("react");
  const { View } = jest.requireActual<typeof import("react-native")>("react-native");
  return {
    Map: forwardRef(function Map({ children }: { children?: React.ReactNode }, _ref) {
      return <View testID="map">{children}</View>;
    }),
    Camera: forwardRef(function Camera(_props, ref) {
      useImperativeHandle(ref, () => mockCamera);
      return null;
    }),
  };
});

const mockFocus = { current: true };
jest.mock("expo-router", () => ({
  ...jest.requireActual("expo-router"),
  useIsFocused: () => mockFocus.current,
}));

jest.mock("../../src/hooks/useBasemapStyle", () => ({
  useBasemapStyle: () => ({ style: { version: 8, sources: {}, layers: [] }, labelFont: ["Noto Sans Regular"] }),
}));

beforeEach(() => {
  mockFocus.current = true;
  mockCamera.setStop.mockClear();
  mockCamera.fitBounds.mockClear();
  useWeatherStore.setState({ latitude: 42.96, longitude: -85.67, recenterNonce: 0, focusBounds: null });
});

test("GPS drift never moves the camera; an explicit recenter keeps the user's zoom", async () => {
  await renderWithProviders(<WeatherMap />);

  await act(async () => useWeatherStore.setState({ latitude: 42.97, longitude: -85.68 }));
  expect(mockCamera.setStop).not.toHaveBeenCalled();

  await act(async () => useWeatherStore.getState().requestRecenter());
  expect(mockCamera.setStop).toHaveBeenCalledTimes(1);
  const stop = mockCamera.setStop.mock.calls[0][0];
  expect(stop.center).toEqual([-85.68, 42.97]);
  expect(stop).not.toHaveProperty("zoom");
});

test("zoom controls are labelled buttons with 44-point targets", async () => {
  await renderWithProviders(<WeatherMap />);
  for (const name of ["Zoom in", "Zoom out"]) {
    const button = screen.getByRole("button", { name });
    const style = StyleSheet.flatten(button.props.style);
    expect(style.minWidth).toBeGreaterThanOrEqual(44);
    expect(style.minHeight).toBeGreaterThanOrEqual(44);
  }
  await fireEvent.press(screen.getByRole("button", { name: "Zoom in" }));
  expect(mockCamera.setStop).toHaveBeenCalledWith({ zoom: 8, duration: 220 });
});

test("an alert's area is framed once, then the request is dropped", async () => {
  await renderWithProviders(<WeatherMap />);
  const bounds = [-86, 42, -85, 43] as [number, number, number, number];
  await act(async () => useWeatherStore.getState().setFocusBounds(bounds));
  expect(mockCamera.fitBounds).toHaveBeenCalledTimes(1);
  expect(mockCamera.fitBounds.mock.calls[0][0]).toEqual(bounds);
  expect(useWeatherStore.getState().focusBounds).toBeNull();
});

test("a recenter requested while the map is hidden is applied when it comes on screen", async () => {
  // Native tabs keep the radar map mounted while another tab is showing.
  mockFocus.current = false;
  const { rerender } = await renderWithProviders(<WeatherMap />);
  await act(async () => {
    useWeatherStore.setState({ latitude: 42.36, longitude: -71.06 });
    useWeatherStore.getState().requestRecenter();
  });
  expect(mockCamera.setStop).not.toHaveBeenCalled();

  mockFocus.current = true;
  await rerender(<WeatherMap />);
  expect(mockCamera.setStop).toHaveBeenCalledTimes(1);
  expect(mockCamera.setStop.mock.calls[0][0].center).toEqual([-71.06, 42.36]);

  // Focusing again without a new request doesn't move the camera again.
  await rerender(<WeatherMap />);
  expect(mockCamera.setStop).toHaveBeenCalledTimes(1);
});
