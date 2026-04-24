import { Stack } from "expo-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { StatusBar } from "expo-status-bar";
import { StyleSheet, LogBox } from "react-native";

LogBox.ignoreLogs(["this._listeners.forEach"]);

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      gcTime: 10 * 60_000,
    },
  },
});

export default function RootLayout() {
  return (
    <GestureHandlerRootView style={styles.container}>
      <QueryClientProvider client={queryClient}>
        <StatusBar style="light" />
        <Stack screenOptions={{ headerShown: false }}>
          <Stack.Screen name="(tabs)" />
          <Stack.Screen
            name="alert/[id]"
            options={{
              presentation: "modal",
              headerShown: true,
              headerTitle: "Alert Details",
            }}
          />
        </Stack>
      </QueryClientProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
});
