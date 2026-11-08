import { Text, View } from "react-native";
import { useQueries } from "@tanstack/react-query";
import { useIsFocused } from "expo-router";
import { fetchOutlook } from "../../lib/api";
import { outlookDays } from "../../lib/spc";
import { useSpcNow } from "../../hooks/useSpcNow";
import { useWeatherStore } from "../../stores/useWeatherStore";

export function PointOutlooks({ lat, lon }: { lat: number; lon: number }) {
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const focused = useIsFocused();
  const now = useSpcNow(focused);
  const queries = useQueries({ queries: ([1, 2, 3] as const).map((day) => ({
    queryKey: ["spc-outlook", serverUrl, day],
    queryFn: ({ signal }: { signal: AbortSignal }) => fetchOutlook(serverUrl, day, signal),
    enabled: focused,
    staleTime: 600_000,
    refetchInterval: 60_000,
  })) });
  const lines = outlookDays(queries.map((q) => q.data), [lon, lat], now);
  return <View accessibilityLabel="SPC outlook at inspected point" style={{ marginTop: 8, gap: 3 }}>
    {lines.map((line, i) => <Text key={i} style={{ color: "#4c5770", fontSize: 12 }}>
      {queries[i].isPending ? `Day ${i + 1}: Loading outlook…` : line}
    </Text>)}
  </View>;
}
