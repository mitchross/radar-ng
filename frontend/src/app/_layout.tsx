// animatedFix patches RN's AnimatedNode.__callListeners and must run before
// anything constructs an Animated node — keep it as the very first import.
import "../lib/animatedFix";
// Telemetry must be imported next so OTEL providers are registered before
// any component code runs fetch() or starts a span.
import { logEvent } from "../lib/telemetry";

import { Stack } from "expo-router";
import {
  QueryClient,
  QueryClientProvider,
  QueryCache,
  MutationCache,
} from "@tanstack/react-query";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { StatusBar } from "expo-status-bar";
import { StyleSheet, View } from "react-native";
import {
  WeatherClearThemeProvider,
  useWeatherClearTheme,
} from "../theme/WeatherClearThemeProvider";

// Root-level error boundary: without it, a single throw anywhere in the tree
// (a Skia worklet edge case, a MapLibre native error surfacing in JS) takes
// down the whole app with a red screen. expo-router's built-in boundary
// shows the error with a retry affordance instead.
export { ErrorBoundary } from "expo-router";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      // 5 min: weather data is stale after minutes anyway, and inactive
      // queries (manifest refetches every 30s, per-location forecasts)
      // otherwise pile up in memory on low-end devices.
      gcTime: 5 * 60_000,
    },
  },
  queryCache: new QueryCache({
    onError: (err, query) => {
      logEvent("error", `query failed: ${(err as Error).message}`, {
        "query.key": JSON.stringify(query.queryKey),
      });
    },
  }),
  mutationCache: new MutationCache({
    onError: (err) => {
      logEvent("error", `mutation failed: ${(err as Error).message}`);
    },
  }),
});

export default function RootLayout() {
  return (
    <GestureHandlerRootView style={styles.container}>
      <SafeAreaProvider>
        <QueryClientProvider client={queryClient}>
          <WeatherClearThemeProvider>
            <ThemedApp />
          </WeatherClearThemeProvider>
        </QueryClientProvider>
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

function ThemedApp() {
  const { resolvedAppearance, theme } = useWeatherClearTheme();
  return (
    <View style={[styles.container, { backgroundColor: theme.colors.canvas }]}>
      <StatusBar style={resolvedAppearance === "dark" ? "light" : "dark"} />
      <Stack
        screenOptions={{
          headerShown: false,
          contentStyle: { backgroundColor: theme.colors.canvas },
        }}
      >
        <Stack.Screen name="(tabs)" />
        <Stack.Screen
          name="alert/[id]"
          options={{ presentation: "modal", headerShown: false }}
        />
      </Stack>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
});
