/**
 * Native tabs — Home · Nowcast · Radar · Alerts · Settings.
 * iOS 26+ draws the Liquid Glass tab bar with SF Symbols; Android draws a
 * Material bottom bar with Material Symbols. The bar hides on the
 * full-screen Radar route.
 */
import { useSegments } from "expo-router";
import { NativeTabs } from "expo-router/native-tabs";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";

export default function TabLayout() {
  const segments = useSegments();
  const onRadar = segments[segments.length - 1] === "radar";
  const alertCount = useAlerts().data?.features?.length ?? 0;
  const { theme } = useWeatherClearTheme();

  return (
    <NativeTabs
      hidden={onRadar}
      minimizeBehavior="onScrollDown"
      // Material hides inactive labels with more than three tabs; keep all five named.
      labelVisibilityMode="labeled"
      tintColor={theme.colors.accent}
      badgeBackgroundColor={theme.colors.destructive}
    >
      <NativeTabs.Trigger name="index">
        <NativeTabs.Trigger.Icon sf={{ default: "house", selected: "house.fill" }} md="home" />
        <NativeTabs.Trigger.Label>Home</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="nowcast">
        <NativeTabs.Trigger.Icon sf={{ default: "cloud.rain", selected: "cloud.rain.fill" }} md="rainy" />
        <NativeTabs.Trigger.Label>Nowcast</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="radar">
        <NativeTabs.Trigger.Icon sf={{ default: "map", selected: "map.fill" }} md="radar" />
        <NativeTabs.Trigger.Label>Radar</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="alerts">
        <NativeTabs.Trigger.Icon
          sf={{ default: "exclamationmark.triangle", selected: "exclamationmark.triangle.fill" }}
          md="warning"
        />
        <NativeTabs.Trigger.Label>Alerts</NativeTabs.Trigger.Label>
        {/* Rendered only with a count: the preview's Badge `hidden` still showed "0". */}
        {alertCount > 0 ? (
          <NativeTabs.Trigger.Badge>{alertCount > 9 ? "9+" : String(alertCount)}</NativeTabs.Trigger.Badge>
        ) : null}
      </NativeTabs.Trigger>
      <NativeTabs.Trigger name="settings">
        <NativeTabs.Trigger.Icon sf={{ default: "gearshape", selected: "gearshape.fill" }} md="settings" />
        <NativeTabs.Trigger.Label>Settings</NativeTabs.Trigger.Label>
      </NativeTabs.Trigger>
    </NativeTabs>
  );
}
