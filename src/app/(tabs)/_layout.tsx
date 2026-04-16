import { View, StyleSheet } from "react-native";
import { Tabs } from "expo-router";

// Custom tab icons — small View-based icons instead of emoji
function SunIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.sunOuter}>
      <View style={[tabIconStyles.sunCenter, { backgroundColor: color }]} />
      {/* Rays */}
      {[0, 45, 90, 135].map((deg) => (
        <View
          key={deg}
          style={[
            tabIconStyles.sunRay,
            { backgroundColor: color, transform: [{ rotate: `${deg}deg` }] },
          ]}
        />
      ))}
    </View>
  );
}

function RadarIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.radarOuter}>
      <View style={[tabIconStyles.radarArc1, { borderColor: color }]} />
      <View style={[tabIconStyles.radarArc2, { borderColor: color }]} />
      <View style={[tabIconStyles.radarDot, { backgroundColor: color }]} />
    </View>
  );
}

function GearIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.gearOuter}>
      <View style={[tabIconStyles.gearRing, { borderColor: color }]} />
      <View style={[tabIconStyles.gearCenter, { backgroundColor: color }]} />
      {/* Teeth */}
      {[0, 60, 120].map((deg) => (
        <View
          key={deg}
          style={[
            tabIconStyles.gearTooth,
            { backgroundColor: color, transform: [{ rotate: `${deg}deg` }] },
          ]}
        />
      ))}
    </View>
  );
}

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: "rgba(13, 17, 23, 0.95)",
          borderTopColor: "rgba(255,255,255,0.08)",
          borderTopWidth: StyleSheet.hairlineWidth,
          height: 56,
          paddingBottom: 6,
          paddingTop: 4,
        },
        tabBarActiveTintColor: "#42A5F5",
        tabBarInactiveTintColor: "rgba(255,255,255,0.35)",
        tabBarLabelStyle: {
          fontSize: 11,
          fontWeight: "600",
          letterSpacing: 0.3,
        },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Weather",
          tabBarIcon: ({ color }) => <SunIcon color={color} />,
        }}
      />
      <Tabs.Screen
        name="radar"
        options={{
          title: "Radar",
          tabBarIcon: ({ color }) => <RadarIcon color={color} />,
        }}
      />
      <Tabs.Screen
        name="settings"
        options={{
          title: "Settings",
          tabBarIcon: ({ color }) => <GearIcon color={color} />,
        }}
      />
    </Tabs>
  );
}

const tabIconStyles = StyleSheet.create({
  // Sun icon
  sunOuter: {
    width: 22,
    height: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  sunCenter: {
    width: 10,
    height: 10,
    borderRadius: 5,
    position: "absolute",
  },
  sunRay: {
    position: "absolute",
    width: 2,
    height: 20,
    borderRadius: 1,
    opacity: 0.6,
  },
  // Radar icon
  radarOuter: {
    width: 22,
    height: 22,
    alignItems: "center",
    justifyContent: "flex-end",
  },
  radarArc1: {
    position: "absolute",
    bottom: 0,
    width: 20,
    height: 10,
    borderTopLeftRadius: 10,
    borderTopRightRadius: 10,
    borderWidth: 2,
    borderBottomWidth: 0,
  },
  radarArc2: {
    position: "absolute",
    bottom: 0,
    width: 12,
    height: 6,
    borderTopLeftRadius: 6,
    borderTopRightRadius: 6,
    borderWidth: 2,
    borderBottomWidth: 0,
  },
  radarDot: {
    width: 4,
    height: 4,
    borderRadius: 2,
    marginBottom: 0,
  },
  // Gear icon
  gearOuter: {
    width: 22,
    height: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  gearRing: {
    position: "absolute",
    width: 14,
    height: 14,
    borderRadius: 7,
    borderWidth: 2,
  },
  gearCenter: {
    position: "absolute",
    width: 4,
    height: 4,
    borderRadius: 2,
  },
  gearTooth: {
    position: "absolute",
    width: 2,
    height: 20,
    borderRadius: 1,
    opacity: 0.7,
  },
});
