import { fireEvent, screen } from "@testing-library/react-native";
import { StyleSheet } from "react-native";
import { MapStylePicker } from "../../src/components/map/MapStylePicker";
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
  useWeatherStore.setState({ mapStyle: "light", activeLayer: "radar", latitude: null, longitude: null });
});

describe("map style picker", () => {
  test("offers each style as a checked radio with a 44-point target", async () => {
    await renderWithProviders(<MapStylePicker visible onClose={jest.fn()} />);
    const radios = screen.getAllByRole("radio");
    expect(radios.map((r) => r.props.accessibilityLabel)).toEqual([
      "Light map style",
      "Dark map style",
      "Satellite map style",
    ]);
    expect(screen.getByRole("radio", { name: "Light map style" })).toBeChecked();
    for (const radio of radios) {
      expect(StyleSheet.flatten(radio.props.style).minHeight).toBeGreaterThanOrEqual(44);
    }
    expect(screen.getByRole("button", { name: "Close map style picker" })).toBeOnTheScreen();
  });

  test("choosing a style applies it and closes the sheet", async () => {
    const onClose = jest.fn();
    await renderWithProviders(<MapStylePicker visible onClose={onClose} />);
    await fireEvent.press(screen.getByRole("radio", { name: "Dark map style" }));
    expect(useWeatherStore.getState().mapStyle).toBe("dark");
    expect(onClose).toHaveBeenCalled();
  });
});

describe("radar buttons", () => {
  test("every map control is a labelled button", async () => {
    await renderWithProviders(
      <RadarFABs inspectorActive={false} onToggleInspector={jest.fn()} onOpenStylePicker={jest.fn()} />,
    );
    for (const name of [
      "Choose radar layer",
      "Toggle storm and lightning overlays",
      "Center map on your location",
      "Choose map style",
      "Refresh radar",
    ]) {
      expect(screen.getByRole("button", { name })).toBeOnTheScreen();
    }
  });

  test("the layer picker lists layers as radios and switches the active one", async () => {
    await renderWithProviders(<RadarFABs inspectorActive={false} onToggleInspector={jest.fn()} />);
    await fireEvent.press(screen.getByRole("button", { name: "Choose radar layer" }));
    const radios = screen.getAllByRole("radio");
    expect(radios.length).toBeGreaterThan(1);
    const target = radios.find((r) => !r.props.accessibilityState?.checked)!;
    await fireEvent.press(target);
    expect(useWeatherStore.getState().activeLayer).not.toBe("radar");
  });
});
