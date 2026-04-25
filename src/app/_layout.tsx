// animatedFix patches RN's AnimatedNode.__callListeners and must run before
// anything constructs an Animated node — keep it as the very first import.
import "../lib/animatedFix";
// Telemetry must be imported next so OTEL providers are registered before
// any component code runs fetch() or starts a span.
import "../lib/telemetry";
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
import { StyleSheet } from "react-native";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 2,
      gcTime: 10 * 60_000,
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
      </SafeAreaProvider>
    </GestureHandlerRootView>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
});
