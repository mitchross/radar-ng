/**
 * Cumulus 5-tab bar — Home · Nowcast · Radar · Alerts · Settings.
 * Matches design_handoff_radar_app: persistent dark translucent pill, hidden on Radar.
 */
import { View, StyleSheet, Text, Pressable } from "react-native";
import type { ComponentProps } from "react";
import { Tabs } from "expo-router";
import { cumulus } from "../../lib/cumulusTheme";
import { useAlerts } from "../../hooks/useAlerts";

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
              android_ripple={{ color: "rgba(255,255,255,0.08)", borderless: true, radius: 40 }}
              accessibilityRole="tab"
              accessibilityState={{ selected: active }}
            >
              <View style={[bar.iconWrap, active && bar.iconWrapActive]}>
                <Icon active={active} />
                {showBadge && (
                  <View style={bar.badge}>
                    <Text style={bar.badgeText}>{alertCount > 9 ? "9+" : alertCount}</Text>
                  </View>
                )}
              </View>
              <Text style={[bar.label, active && bar.labelActive]}>{label}</Text>
            </Pressable>
          );
        })}
      </View>
    </View>
  );
}

const ICON_COLOR = cumulus.inkMuted;
const ICON_COLOR_ACTIVE = "#FFFFFF";
const col = (active: boolean) => (active ? ICON_COLOR_ACTIVE : ICON_COLOR);

function HomeIcon({ active }: { active: boolean }) {
  const c = col(active);
  return (
    <View style={icon.box}>
      <View style={[icon.homeRoof, { borderBottomColor: c }]} />
      <View style={[icon.homeBody, { backgroundColor: c }]} />
      <View style={[icon.homeDoor, { backgroundColor: active ? cumulus.accent : "rgba(0,0,0,0.45)" }]} />
    </View>
  );
}

function NowcastIcon({ active }: { active: boolean }) {
  const c = col(active);
  return (
    <View style={icon.box}>
      <View style={[icon.bar, icon.bar1, { backgroundColor: c }]} />
      <View style={[icon.bar, icon.bar2, { backgroundColor: c }]} />
      <View style={[icon.bar, icon.bar3, { backgroundColor: c }]} />
      <View style={[icon.bar, icon.bar4, { backgroundColor: c }]} />
    </View>
  );
}

function RadarIcon({ active }: { active: boolean }) {
  const c = col(active);
  return (
    <View style={icon.box}>
      <View style={[icon.radarOuter, { borderColor: c }]} />
      <View style={[icon.radarMid, { borderColor: c }]} />
      <View style={[icon.radarInner, { backgroundColor: c }]} />
      <View style={[icon.radarSweep, { backgroundColor: c, opacity: active ? 0.9 : 0.5 }]} />
    </View>
  );
}

function AlertsIcon({ active }: { active: boolean }) {
  const c = col(active);
  return (
    <View style={icon.box}>
      <View style={[icon.alertTri, { borderBottomColor: c }]} />
      <View style={[icon.alertBar, { backgroundColor: active ? cumulus.alert : "rgba(0,0,0,0.55)" }]} />
      <View style={[icon.alertDot, { backgroundColor: active ? cumulus.alert : "rgba(0,0,0,0.55)" }]} />
    </View>
  );
}

function SettingsIcon({ active }: { active: boolean }) {
  const c = col(active);
  return (
    <View style={icon.box}>
      <View style={[icon.gearRing, { borderColor: c }]} />
      <View style={[icon.gearDot, { backgroundColor: active ? cumulus.accent : c }]} />
    </View>
  );
}

const ICONS: Record<string, React.ComponentType<{ active: boolean }>> = {
  index: HomeIcon,
  nowcast: NowcastIcon,
  radar: RadarIcon,
  alerts: AlertsIcon,
  settings: SettingsIcon,
};

const bar = StyleSheet.create({
  wrap: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    paddingHorizontal: 10,
    paddingBottom: 18,
    paddingTop: 8,
  },
  pill: {
    flexDirection: "row",
    backgroundColor: "rgba(10,10,20,0.78)",
    borderRadius: 26,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.08)",
    paddingVertical: 10,
    paddingHorizontal: 6,
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 10 },
    shadowOpacity: 0.45,
    shadowRadius: 22,
    elevation: 14,
  },
  item: { flex: 1, alignItems: "center", justifyContent: "center", paddingVertical: 4 },
  iconWrap: {
    width: 42,
    height: 30,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 14,
    marginBottom: 3,
  },
  iconWrapActive: {
    backgroundColor: cumulus.accentSoft,
    borderWidth: 1,
    borderColor: cumulus.accentBorder,
  },
  badge: {
    position: "absolute",
    top: -2,
    right: 2,
    minWidth: 14,
    height: 14,
    paddingHorizontal: 3,
    borderRadius: 7,
    backgroundColor: cumulus.alert,
    alignItems: "center",
    justifyContent: "center",
  },
  badgeText: { color: "#fff", fontSize: 9, fontWeight: "800", lineHeight: 12 },
  label: { fontSize: 10.5, fontWeight: "600", color: cumulus.inkMuted, letterSpacing: 0.2 },
  labelActive: { color: "#FFFFFF", fontWeight: "700" },
});

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
