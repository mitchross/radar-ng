/**
 * Cumulus tab bar — Home · Nowcast · Radar.
 * Floating translucent rounded-pill style (matches app.jsx reference).
 */
import { View, StyleSheet } from "react-native";
import { Tabs } from "expo-router";
import { cumulus } from "../../lib/cumulusTheme";

function HomeIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.box}>
      <View style={[tabIconStyles.homeRoofL, { borderRightColor: color }]} />
      <View style={[tabIconStyles.homeRoofR, { borderLeftColor: color }]} />
      <View style={[tabIconStyles.homeBody, { borderColor: color }]} />
    </View>
  );
}

function NowcastIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.box}>
      <View style={[tabIconStyles.chartBar1, { backgroundColor: color }]} />
      <View style={[tabIconStyles.chartBar2, { backgroundColor: color }]} />
      <View style={[tabIconStyles.chartBar3, { backgroundColor: color }]} />
      <View style={[tabIconStyles.chartBar4, { backgroundColor: color }]} />
    </View>
  );
}

function RadarIcon({ color }: { color: string }) {
  return (
    <View style={tabIconStyles.box}>
      <View style={[tabIconStyles.radarOuter, { borderColor: color }]} />
      <View style={[tabIconStyles.radarInner, { borderColor: color }]} />
      <View style={[tabIconStyles.radarDot, { backgroundColor: color }]} />
      <View style={[tabIconStyles.radarSweep, { backgroundColor: color }]} />
    </View>
  );
}

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        // Cumulus floating pill — the outer tabBarStyle is transparent and
        // tabBarBackground renders the rounded card inset from the edges.
        tabBarStyle: {
          position: "absolute",
          left: 0,
          right: 0,
          bottom: 0,
          height: 78,
          backgroundColor: "transparent",
          borderTopWidth: 0,
          elevation: 0,
        },
        tabBarBackground: () => (
          <View pointerEvents="none" style={tabBgStyles.pill} />
        ),
        tabBarItemStyle: {
          marginHorizontal: 16,
          paddingTop: 6,
        },
        tabBarActiveTintColor: cumulus.ink,
        tabBarInactiveTintColor: "rgba(255,255,255,0.48)",
        tabBarLabelStyle: {
          fontSize: 9.5,
          fontWeight: "600",
          letterSpacing: 0.2,
          marginTop: 2,
        },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{ title: "Home", tabBarIcon: ({ color }) => <HomeIcon color={color} /> }}
      />
      <Tabs.Screen
        name="nowcast"
        options={{ title: "Nowcast", tabBarIcon: ({ color }) => <NowcastIcon color={color} /> }}
      />
      <Tabs.Screen
        name="radar"
        options={{ title: "Radar", tabBarIcon: ({ color }) => <RadarIcon color={color} /> }}
      />
    </Tabs>
  );
}

const tabBgStyles = StyleSheet.create({
  pill: {
    position: "absolute",
    left: 16,
    right: 16,
    top: 8,
    bottom: 20,
    backgroundColor: "rgba(10,10,20,0.92)",
    borderRadius: 22,
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.12)",
    shadowColor: "#000",
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.35,
    shadowRadius: 20,
    elevation: 12,
  },
});

const tabIconStyles = StyleSheet.create({
  box: { width: 22, height: 22, alignItems: "center", justifyContent: "center" },
  // Home — simple house silhouette
  homeBody: {
    position: "absolute",
    bottom: 2,
    width: 14,
    height: 9,
    borderWidth: 1.5,
    borderTopWidth: 0,
    borderBottomLeftRadius: 1,
    borderBottomRightRadius: 1,
  },
  homeRoofL: {
    position: "absolute",
    top: 3,
    left: 2,
    width: 0,
    height: 0,
    borderRightWidth: 9,
    borderTopWidth: 0,
    borderBottomWidth: 8,
    borderRightColor: "transparent",
    borderBottomColor: "transparent",
  },
  homeRoofR: {
    position: "absolute",
    top: 3,
    right: 2,
    width: 0,
    height: 0,
    borderLeftWidth: 9,
    borderTopWidth: 0,
    borderBottomWidth: 8,
    borderLeftColor: "transparent",
    borderBottomColor: "transparent",
  },
  // Nowcast — ascending bars
  chartBar1: { position: "absolute", bottom: 3, left: 2, width: 3, height: 6, borderRadius: 1 },
  chartBar2: { position: "absolute", bottom: 3, left: 7, width: 3, height: 10, borderRadius: 1 },
  chartBar3: { position: "absolute", bottom: 3, left: 12, width: 3, height: 14, borderRadius: 1 },
  chartBar4: { position: "absolute", bottom: 3, left: 17, width: 3, height: 8, borderRadius: 1 },
  // Radar — concentric circles + sweep line
  radarOuter: {
    position: "absolute",
    width: 18,
    height: 18,
    borderRadius: 9,
    borderWidth: 1.3,
  },
  radarInner: {
    position: "absolute",
    width: 10,
    height: 10,
    borderRadius: 5,
    borderWidth: 1.1,
    opacity: 0.6,
  },
  radarDot: {
    position: "absolute",
    width: 3,
    height: 3,
    borderRadius: 1.5,
  },
  radarSweep: {
    position: "absolute",
    left: 11,
    top: 4,
    width: 1.5,
    height: 7,
    borderRadius: 1,
    transform: [{ rotate: "35deg" }, { translateY: -1 }],
  },
});
