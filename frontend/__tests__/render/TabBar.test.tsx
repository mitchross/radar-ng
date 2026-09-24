import { screen } from "@testing-library/react-native";
import TabLayout from "../../src/app/(tabs)/_layout";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

// The native tab bar is UIKit/Material; this stand-in renders what the layout
// declares so the test can read tabs, icons, badges and visibility.
jest.mock("expo-router/native-tabs", () => {
  const { Text, View } = jest.requireActual<typeof import("react-native")>("react-native");
  const Trigger = ({ name, children }: { name: string; children: React.ReactNode }) => (
    <View testID={`tab-${name}`}>{children}</View>
  );
  Trigger.Icon = ({ sf, md }: { sf: { default: string; selected: string }; md: string }) => (
    <Text testID="icon">{`${sf.default}|${sf.selected}|${md}`}</Text>
  );
  Trigger.Label = ({ children }: { children: string }) => <Text testID="label">{children}</Text>;
  Trigger.Badge = ({ hidden, children }: { hidden?: boolean; children: string }) =>
    hidden ? null : <Text testID="badge">{children}</Text>;
  const NativeTabs = ({ hidden, children }: { hidden?: boolean; children: React.ReactNode }) => (
    <View testID="tabs" accessibilityState={{ expanded: !hidden }}>{children}</View>
  );
  NativeTabs.Trigger = Trigger;
  return { NativeTabs };
});

const mockSegments = { current: ["(tabs)"] as string[] };
jest.mock("expo-router", () => ({ useSegments: () => mockSegments.current }));

const mockAlerts = { current: 0 };
jest.mock("../../src/hooks/useAlerts", () => ({
  useAlerts: () => ({ data: { features: Array.from({ length: mockAlerts.current }) } }),
}));

beforeEach(() => {
  mockSegments.current = ["(tabs)"];
  mockAlerts.current = 0;
  useWeatherStore.setState({ latitude: null, longitude: null });
});

test("five tabs, in order, each with an SF Symbol pair and a Material icon", async () => {
  await renderWithProviders(<TabLayout />);
  expect(screen.getAllByTestId("label").map((l) => l.props.children)).toEqual([
    "Home",
    "Nowcast",
    "Radar",
    "Alerts",
    "Settings",
  ]);
  for (const icon of screen.getAllByTestId("icon")) {
    const [sf, selected, md] = String(icon.props.children).split("|");
    expect(selected).toBe(`${sf}.fill`);
    expect(md).toMatch(/^[a-z_]+$/);
  }
});

test("the Alerts tab shows a count badge only when alerts are active", async () => {
  const { rerender } = await renderWithProviders(<TabLayout />);
  expect(screen.queryByTestId("badge")).toBeNull();
  mockAlerts.current = 12;
  await rerender(<TabLayout />);
  expect(screen.getByTestId("badge")).toHaveTextContent("9+");
});

test("the bar hides only on the full-screen radar route", async () => {
  await renderWithProviders(<TabLayout />);
  expect(screen.getByTestId("tabs")).toBeExpanded();
  mockSegments.current = ["(tabs)", "radar"];
  await renderWithProviders(<TabLayout />);
  expect(screen.getAllByTestId("tabs").at(-1)).toBeCollapsed();
});
