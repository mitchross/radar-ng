import { act, fireEvent, screen, waitFor } from "@testing-library/react-native";
import { Linking } from "react-native";
import { RainAlertButton } from "../../src/components/home/RainAlertButton";
import { useWeatherStore } from "../../src/stores/useWeatherStore";
import { renderWithProviders } from "./helpers";

const mockEnsurePermission = jest.fn();
jest.mock("../../src/lib/rainAlertScheduler", () => ({
  ensureNotificationPermission: (prompt: boolean) => mockEnsurePermission(prompt),
}));

beforeEach(() => {
  mockEnsurePermission.mockReset();
  useWeatherStore.setState({ rainAlertsEnabled: false });
});

describe("Notify me", () => {
  test("enables rain alerts through the permission-checked path and reflects it", async () => {
    mockEnsurePermission.mockResolvedValue(true);
    await renderWithProviders(<RainAlertButton />);
    await act(async () => {
      fireEvent.press(screen.getByRole("button", { name: "Notify me when rain starts" }));
    });
    await waitFor(() => expect(useWeatherStore.getState().rainAlertsEnabled).toBe(true));
    expect(mockEnsurePermission).toHaveBeenCalledWith(true);
    const on = screen.getByRole("button", { name: "Rain alerts are on" });
    expect(on).toBeSelected();
    expect(on).toBeDisabled();
  });

  test("shows the enabled state when alerts were already on in Settings", async () => {
    useWeatherStore.setState({ rainAlertsEnabled: true });
    await renderWithProviders(<RainAlertButton />);
    expect(screen.getByText(/Alerts on/)).toBeOnTheScreen();
    expect(mockEnsurePermission).not.toHaveBeenCalled();
  });

  test("a refused notification permission keeps alerts off and points at system settings", async () => {
    mockEnsurePermission.mockResolvedValue(false);
    const openSettings = jest.spyOn(Linking, "openSettings").mockResolvedValue(undefined);
    await renderWithProviders(<RainAlertButton />);
    await act(async () => {
      fireEvent.press(screen.getByRole("button", { name: "Notify me when rain starts" }));
    });
    await waitFor(() => expect(screen.getByText("Allow notifications")).toBeOnTheScreen());
    expect(useWeatherStore.getState().rainAlertsEnabled).toBe(false);
    await fireEvent.press(screen.getByRole("button", { name: "Allow notifications in system settings" }));
    expect(openSettings).toHaveBeenCalled();
  });
});
