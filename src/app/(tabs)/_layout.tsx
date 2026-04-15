import { Text } from "react-native";
import { Tabs } from "expo-router";

export default function TabLayout() {
  return (
    <Tabs
      screenOptions={{
        headerShown: false,
        tabBarStyle: {
          backgroundColor: "rgba(10, 10, 20, 0.9)",
          borderTopWidth: 0,
          height: 50,
          paddingBottom: 4,
          position: "absolute",
          elevation: 0,
        },
        tabBarActiveTintColor: "#4fc3f7",
        tabBarInactiveTintColor: "#555",
        tabBarLabelStyle: {
          fontSize: 10,
          fontWeight: "600",
        },
      }}
    >
      <Tabs.Screen
        name="index"
        options={{
          title: "Map",
          tabBarIcon: ({ color }) => (
            <Text style={{ fontSize: 18, color }}>{"\uD83D\uDDFA\uFE0F"}</Text>
          ),
        }}
      />
      <Tabs.Screen
        name="settings"
        options={{
          title: "Settings",
          tabBarIcon: ({ color }) => (
            <Text style={{ fontSize: 18, color }}>{"\u2699\uFE0F"}</Text>
          ),
        }}
      />
    </Tabs>
  );
}
