import { fireEvent, screen } from "@testing-library/react-native";
import { StyleSheet } from "react-native";
import { MapOptionsSheet } from "../../src/components/map/MapOptionsSheet";
import { RadarFABs } from "../../src/components/map/RadarFABs";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

// The SwiftUI sheet is native; render its React Native content inline when presented.
jest.mock("@expo/ui", () => ({
  BottomSheet: ({ isPresented, children }: { isPresented: boolean; children: React.ReactNode }) =>
    isPresented ? children : null,
  RNHostView: ({ children }: { children: React.ReactNode }) => children,
}));

beforeEach(() => {
  useWeatherStore.setState({ mapStyle: "light", activeLayer: "radar", extrasVisible: false, latitude: null, longitude: null });
});

describe("map options sheet", () => {
  test("offers layers and map styles as checked radios with 44-point targets", async () => {
    await renderWithProviders(<MapOptionsSheet visible onClose={jest.fn()} />);
    const styles = ["Light map style", "Dark map style", "Satellite map style"];
    for (const name of styles) {
      const radio = screen.getByRole("radio", { name });
      expect(StyleSheet.flatten(radio.props.style).minHeight).toBeGreaterThanOrEqual(44);
    }
    expect(screen.getByRole("radio", { name: "Light map style" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Radar radar layer" })).toBeChecked();
    expect(screen.getByRole("button", { name: "Close map options" })).toBeOnTheScreen();
  });

  test("choosing a map style applies it and keeps the sheet open", async () => {
    const onClose = jest.fn();
    await renderWithProviders(<MapOptionsSheet visible onClose={onClose} />);
    await fireEvent.press(screen.getByRole("radio", { name: "Dark map style" }));
    expect(useWeatherStore.getState().mapStyle).toBe("dark");
    expect(onClose).not.toHaveBeenCalled();
  });

  test("choosing a layer switches it and closes the sheet", async () => {
    const onClose = jest.fn();
    await renderWithProviders(<MapOptionsSheet visible onClose={onClose} />);
    await fireEvent.press(screen.getByRole("radio", { name: "Air Quality radar layer" }));
    expect(useWeatherStore.getState().activeLayer).toBe("air-quality");
    expect(onClose).toHaveBeenCalled();
  });

  test("the storms switch toggles the overlays", async () => {
    await renderWithProviders(<MapOptionsSheet visible onClose={jest.fn()} />);
    await fireEvent(screen.getByLabelText("Toggle storm and lightning overlays"), "valueChange", true);
    expect(useWeatherStore.getState().extrasVisible).toBe(true);
  });
});

describe("radar buttons", () => {
  test("the capsule has map options and locate, both labelled buttons", async () => {
    const onOpen = jest.fn();
    await renderWithProviders(<RadarFABs onOpenMapOptions={onOpen} />);
    expect(screen.getAllByRole("button").map((b) => b.props.accessibilityLabel)).toEqual([
      "Choose radar layer",
      "Center map on your location",
    ]);
    await fireEvent.press(screen.getByRole("button", { name: "Choose radar layer" }));
    expect(onOpen).toHaveBeenCalled();
  });
});
