import { fireEvent, screen } from "@testing-library/react-native";
import { StyleSheet } from "react-native";
import { CumulusTabBar } from "../../src/app/(tabs)/_layout";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

const NAMES = ["index", "nowcast", "radar", "alerts", "settings"];
const TITLES = ["Home", "Nowcast", "Radar", "Alerts", "Settings"];

function tabProps(activeIndex: number) {
  const routes = NAMES.map((name) => ({ key: `${name}-key`, name, params: undefined }));
  const descriptors = Object.fromEntries(
    routes.map((r, i) => [r.key, { options: { title: TITLES[i] } }]),
  );
  const navigation = {
    emit: jest.fn(() => ({ defaultPrevented: false })),
    navigate: jest.fn(),
  };
  const props = { state: { index: activeIndex, routes }, descriptors, navigation };
  return { props: props as unknown as Parameters<typeof CumulusTabBar>[0], navigation };
}

beforeEach(() => {
  // No coordinates: the alerts query stays disabled, so nothing hits the network.
  useWeatherStore.setState({ latitude: null, longitude: null });
});

test("every tab is a labelled, selectable tab at least 44 points tall", async () => {
  await renderWithProviders(<CumulusTabBar {...tabProps(0).props} />);
  const tabs = screen.getAllByRole("tab");
  expect(tabs.map((t) => t.props.accessibilityLabel)).toEqual(TITLES);
  expect(screen.getByRole("tab", { name: "Home" })).toBeSelected();
  expect(screen.getByRole("tab", { name: "Alerts" })).not.toBeSelected();
  for (const tab of tabs) {
    expect(StyleSheet.flatten(tab.props.style).minHeight).toBeGreaterThanOrEqual(44);
  }
});

test("pressing an inactive tab navigates to it", async () => {
  const { props, navigation } = tabProps(0);
  await renderWithProviders(<CumulusTabBar {...props} />);
  await fireEvent.press(screen.getByRole("tab", { name: "Settings" }));
  expect(navigation.navigate).toHaveBeenCalledWith("settings", undefined);
});

test("the bar hides only on the full-screen radar route", async () => {
  await renderWithProviders(<CumulusTabBar {...tabProps(2).props} />);
  expect(screen.queryAllByRole("tab")).toHaveLength(0);
});
