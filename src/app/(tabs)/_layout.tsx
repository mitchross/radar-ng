import { Text } from "react-native";
import { Tabs } from "expo-router";

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: "#0d1117",
          borderTopColor: "#21262d",
          borderTopWidth: 1,
          height: 56,
          paddingBottom: 6,
          paddingTop: 4,
        },
        tabBarActiveTintColor: "#1E88E5",
        tabBarInactiveTintColor: "#484f58",
        tabBarLabelStyle: {
          fontSize: 11,
          fontWeight: "600",
        },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Forecast",
          tabBarIcon: ({ color }) => (
            <Text style={{ fontSize: 20, color }}>{"\u2600\uFE0F"}</Text>
          ),
        }}
      />
      <Tabs.Screen
        name="radar"
        options={{
          title: "Radar",
          tabBarIcon: ({ color }) => (
            <Text style={{ fontSize: 20, color }}>{"\uD83D\uDEF0\uFE0F"}</Text>
          ),
        }}
      />
      <Tabs.Screen
        name="settings"
        options={{
          title: "Settings",
          tabBarIcon: ({ color }) => (
            <Text style={{ fontSize: 20, color }}>{"\u2699\uFE0F"}</Text>
          ),
        }}
      />
    </Tabs>
  );
}
