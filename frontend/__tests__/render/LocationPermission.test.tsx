import { act, fireEvent, screen, waitFor } from "@testing-library/react-native";
import { Linking } from "react-native";
import { LocationPermissionCard } from "../../src/components/home/LocationPermissionCard";
import { useActiveLocation } from "../../src/hooks/useActiveLocation";
import { requestDeviceLocation, useLocationController } from "../../src/hooks/useLocation";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

const mockLocation = {
  getForegroundPermissionsAsync: jest.fn(),
  requestForegroundPermissionsAsync: jest.fn(),
  getLastKnownPositionAsync: jest.fn(),
  getCurrentPositionAsync: jest.fn(),
};
// Resolved per call: the factory runs before `mockLocation` is initialised.
jest.mock("expo-location", () => ({
  getForegroundPermissionsAsync: () => mockLocation.getForegroundPermissionsAsync(),
  requestForegroundPermissionsAsync: () => mockLocation.requestForegroundPermissionsAsync(),
  getLastKnownPositionAsync: () => mockLocation.getLastKnownPositionAsync(),
  getCurrentPositionAsync: () => mockLocation.getCurrentPositionAsync(),
  Accuracy: { Balanced: 3 },
}));

const fix = { coords: { latitude: 42.5, longitude: -85.5 }, timestamp: Date.now() };

beforeEach(() => {
  jest.clearAllMocks();
  useWeatherStore.setState({ locationMode: "device", latitude: null, longitude: null, locationStatus: "locating", lastFixAt: null });
  mockLocation.getLastKnownPositionAsync.mockResolvedValue(fix);
  mockLocation.getCurrentPositionAsync.mockResolvedValue(fix);
});

/** Home's permission card, driven by the real location controller. */
function Harness({ onChooseCity }: { onChooseCity: () => void }) {
  useLocationController();
  const location = useActiveLocation();
  if (!location.permission) return null;
  return (
    <LocationPermissionCard
      kind={location.permission}
      fallbackName="Grand Rapids"
      onAllow={() => void requestDeviceLocation()}
      onChooseCity={onChooseCity}
    />
  );
}

describe("location permission pre-prompt", () => {
  test("launch never raises the OS dialog: an unasked permission shows the pre-prompt over the fallback city", async () => {
    mockLocation.getForegroundPermissionsAsync.mockResolvedValue({ status: "undetermined", canAskAgain: true });
    await renderWithProviders(<Harness onChooseCity={jest.fn()} />);
    await waitFor(() => expect(screen.getByText("Weather for where you are?")).toBeOnTheScreen());
    expect(mockLocation.requestForegroundPermissionsAsync).not.toHaveBeenCalled();
    const state = useWeatherStore.getState();
    expect(state.locationStatus).toBe("undetermined");
    expect(state.latitude).toBeCloseTo(42.9634);
    expect(screen.getByRole("button", { name: "Allow location" })).toBeOnTheScreen();
    expect(screen.getByRole("button", { name: "Choose a city" })).toBeOnTheScreen();
  });

  test("Allow raises the OS dialog and, once granted, replaces the fallback with a device fix", async () => {
    mockLocation.getForegroundPermissionsAsync.mockResolvedValue({ status: "undetermined", canAskAgain: true });
    mockLocation.requestForegroundPermissionsAsync.mockResolvedValue({ status: "granted", canAskAgain: true });
    await renderWithProviders(<Harness onChooseCity={jest.fn()} />);
    await waitFor(() => expect(screen.getByText("Weather for where you are?")).toBeOnTheScreen());
    await act(async () => {
      fireEvent.press(screen.getByRole("button", { name: "Allow location" }));
    });
    await waitFor(() => expect(useWeatherStore.getState().locationStatus).toBe("live"));
    expect(mockLocation.requestForegroundPermissionsAsync).toHaveBeenCalledTimes(1);
    expect(useWeatherStore.getState().latitude).toBe(42.5);
    expect(screen.queryByTestId("location-permission-card")).toBeNull();
  });

  test("a granted permission takes a fix without any card", async () => {
    mockLocation.getForegroundPermissionsAsync.mockResolvedValue({ status: "granted", canAskAgain: true });
    await renderWithProviders(<Harness onChooseCity={jest.fn()} />);
    await waitFor(() => expect(useWeatherStore.getState().locationStatus).toBe("live"));
    expect(screen.queryByTestId("location-permission-card")).toBeNull();
  });
});

describe("after a denial", () => {
  test("the card offers Open Settings and Choose a city", async () => {
    mockLocation.getForegroundPermissionsAsync.mockResolvedValue({ status: "denied", canAskAgain: false });
    const openSettings = jest.spyOn(Linking, "openSettings").mockResolvedValue(undefined);
    const onChooseCity = jest.fn();
    await renderWithProviders(<Harness onChooseCity={onChooseCity} />);
    await waitFor(() => expect(screen.getByText("Location is off")).toBeOnTheScreen());
    expect(useWeatherStore.getState().locationStatus).toBe("denied");
    await fireEvent.press(screen.getByRole("button", { name: "Open Settings" }));
    expect(openSettings).toHaveBeenCalled();
    await fireEvent.press(screen.getByRole("button", { name: "Choose a city" }));
    expect(onChooseCity).toHaveBeenCalled();
  });
});
