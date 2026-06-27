/**
 * Weather Clear 5-tab bar — Home · Nowcast · Radar · Alerts · Settings.
 * Uses the editorial theme and remains hidden on the full-screen Radar route.
 */
import { View, StyleSheet, Text, Pressable } from "react-native";
import { useMemo, type ComponentProps } from "react";
import { Tabs } from "expo-router";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";

type ExpoTabBarProps = Parameters<NonNullable<ComponentProps<typeof Tabs>["tabBar"]>>[0];

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{ headerShown: false }}
      tabBar={(props) => <CumulusTabBar {...props} />}
    >
      <Tabs.Screen name="index" options={{ title: "Home" }} />
      <Tabs.Screen name="nowcast" options={{ title: "Nowcast" }} />
      <Tabs.Screen name="radar" options={{ title: "Radar" }} />
      <Tabs.Screen name="alerts" options={{ title: "Alerts" }} />
      <Tabs.Screen name="settings" options={{ title: "Settings" }} />
    </Tabs>
  );
}

function CumulusTabBar({ state, descriptors, navigation }: ExpoTabBarProps) {
  const activeRoute = state.routes[state.index]?.name;
  const alertsQuery = useAlerts();
  const alertCount = alertsQuery.data?.features?.length ?? 0;
  const { bottom } = useSafeAreaInsets();
  const { theme } = useWeatherClearTheme();
  const bar = useMemo(() => createBarStyles(theme, bottom), [bottom, theme]);

  // Hide on Radar for full-bleed map
  if (activeRoute === "radar") return null;

  return (
    <View pointerEvents="box-none" style={bar.wrap}>
      <View style={bar.pill}>
        {state.routes.map((route, idx) => {
          const { options } = descriptors[route.key];
          const label = typeof options.title === "string" ? options.title : route.name;
          const active = state.index === idx;
          const onPress = () => {
            const event = navigation.emit({
              type: "tabPress",
              target: route.key,
              canPreventDefault: true,
            });
            if (!active && !event.defaultPrevented) {
              navigation.navigate(route.name, route.params);
            }
          };
          const Icon = ICONS[route.name] ?? HomeIcon;
          const showBadge = route.name === "alerts" && alertCount > 0;
          return (
            <Pressable
              key={route.key}
              onPress={onPress}
              style={bar.item}
              android_ripple={{ color: theme.colors.accentSoft, borderless: true, radius: 40 }}
              accessibilityRole="tab"
              accessibilityLabel={label}
              accessibilityState={{ selected: active }}
            >
              <View style={bar.iconWrap}>
                <Icon
                  active={active}
                  color={active ? theme.colors.accent : theme.colors.textFaint}
                  mutedDetail={theme.colors.border}
                />
                {showBadge ? (
                  <View style={[bar.badge, { backgroundColor: theme.colors.destructive }]}>
                    <Text style={bar.badgeText}>{alertCount > 9 ? "9+" : alertCount}</Text>
                  </View>
                ) : null}
              </View>
              <Text style={[bar.label, active ? bar.labelActive : null]}>{label}</Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

type TabIconProps = {
  active: boolean;
  color: string;
  mutedDetail: string;
};

function HomeIcon({ active, color, mutedDetail }: TabIconProps) {
  return (
    <View style={icon.box}>
      <View style={[icon.homeRoof, { borderBottomColor: color }]} />
      <View style={[icon.homeBody, { backgroundColor: color }]} />
      <View style={[icon.homeDoor, { backgroundColor: active ? "#ffffff" : mutedDetail }]} />
    </View>
  );
}

function NowcastIcon({ color }: TabIconProps) {
  return (
    <View style={icon.box}>
      <View style={[icon.bar, icon.bar1, { backgroundColor: color }]} />
      <View style={[icon.bar, icon.bar2, { backgroundColor: color }]} />
      <View style={[icon.bar, icon.bar3, { backgroundColor: color }]} />
      <View style={[icon.bar, icon.bar4, { backgroundColor: color }]} />
    </View>
  );
}

function RadarIcon({ active, color }: TabIconProps) {
  return (
    <View style={icon.box}>
      <View style={[icon.radarOuter, { borderColor: color }]} />
      <View style={[icon.radarMid, { borderColor: color }]} />
      <View style={[icon.radarInner, { backgroundColor: color }]} />
      <View style={[icon.radarSweep, { backgroundColor: color, opacity: active ? 0.9 : 0.5 }]} />
    </View>
  );
}

function AlertsIcon({ active, color, mutedDetail }: TabIconProps) {
  return (
    <View style={icon.box}>
      <View style={[icon.alertTri, { borderBottomColor: color }]} />
      <View style={[icon.alertBar, { backgroundColor: active ? "#ffffff" : mutedDetail }]} />
      <View style={[icon.alertDot, { backgroundColor: active ? "#ffffff" : mutedDetail }]} />
    </View>
  );
}

function SettingsIcon({ active, color, mutedDetail }: TabIconProps) {
  return (
    <View style={icon.box}>
      <View style={[icon.gearRing, { borderColor: color }]} />
      <View style={[icon.gearDot, { backgroundColor: active ? color : mutedDetail }]} />
    </View>
  );
}

const ICONS: Record<string, React.ComponentType<TabIconProps>> = {
  index: HomeIcon,
  nowcast: NowcastIcon,
  radar: RadarIcon,
  alerts: AlertsIcon,
  settings: SettingsIcon,
};

function createBarStyles(theme: WeatherClearTheme, bottom: number) {
  return StyleSheet.create({
  wrap: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
  },
  pill: {
    flexDirection: "row",
    backgroundColor: theme.colors.canvas,
    borderTopWidth: 1,
    borderTopColor: theme.colors.divider,
    paddingTop: 6,
    paddingBottom: Math.max(bottom, 8),
    paddingHorizontal: 12,
  },
  item: { flex: 1, minHeight: 44, alignItems: "center", justifyContent: "center", paddingVertical: 3 },
  iconWrap: {
    width: 42,
    height: 30,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 14,
    marginBottom: 2,
  },
  iconWrapActive: {},
  badge: {
    position: "absolute",
    top: -2,
    right: 2,
    minWidth: 14,
    height: 14,
    paddingHorizontal: 3,
    borderRadius: 7,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: { color: "#fff", fontSize: 9, fontWeight: "800", lineHeight: 12 },
  label: {
    fontFamily: theme.typography.uiSemibold,
    fontSize: 10,
    fontWeight: "600",
    color: theme.colors.textFaint,
    letterSpacing: 0.2,
  },
  labelActive: { color: theme.colors.accent, fontFamily: theme.typography.uiBold },
  });
}

const icon = StyleSheet.create({
  box: { width: 22, height: 22, alignItems: "center", justifyContent: "center" },

  homeRoof: {
    position: "absolute", top: 2, width: 0, height: 0,
    borderLeftWidth: 11, borderRightWidth: 11, borderBottomWidth: 8,
    borderLeftColor: "transparent", borderRightColor: "transparent",
  },
  homeBody: { position: "absolute", bottom: 2, width: 16, height: 11, borderRadius: 2 },
  homeDoor: { position: "absolute", bottom: 2, width: 4, height: 6, borderTopLeftRadius: 1.5, borderTopRightRadius: 1.5 },

  bar: { position: "absolute", bottom: 3, width: 3, borderRadius: 1.5 },
  bar1: { left: 1, height: 6 },
  bar2: { left: 6, height: 10 },
  bar3: { left: 11, height: 14 },
  bar4: { left: 16, height: 8 },

  radarOuter: { position: "absolute", width: 20, height: 20, borderRadius: 10, borderWidth: 1.4 },
  radarMid: { position: "absolute", width: 12, height: 12, borderRadius: 6, borderWidth: 1.2, opacity: 0.7 },
  radarInner: { position: "absolute", width: 3.5, height: 3.5, borderRadius: 1.75 },
  radarSweep: {
    position: "absolute", width: 1.5, height: 9, top: 2, borderRadius: 1,
    transform: [{ rotate: "38deg" }, { translateY: 3 }],
  },

  alertTri: {
    position: "absolute", top: 2,
    width: 0, height: 0,
    borderLeftWidth: 10, borderRightWidth: 10, borderBottomWidth: 16,
    borderLeftColor: "transparent", borderRightColor: "transparent",
  },
  alertBar: { position: "absolute", bottom: 7, width: 2, height: 6, borderRadius: 1 },
  alertDot: { position: "absolute", bottom: 4, width: 2.5, height: 2.5, borderRadius: 1.25 },

  gearRing: { width: 16, height: 16, borderRadius: 8, borderWidth: 2 },
  gearDot: { position: "absolute", width: 5, height: 5, borderRadius: 2.5 },
});
