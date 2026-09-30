/**
 * Nationwide storm-based NWS warning polygons (/api/alerts/map), coloured by
 * event like the web site. Alerts already drawn by AlertPolygon (your point's
 * alerts) are skipped so a polygon is never filled twice.
 *
 * Always mounted; empty collection (and no polling) while `extrasVisible` is off.
 */
import { useMemo } from "react";
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useQuery } from "@tanstack/react-query";
import { useIsFocused } from "expo-router";
import { fetchMapAlerts } from "../../lib/api";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export const WARNING_COLORS: [string, string][] = [
  ["Tornado Warning", "#ff1f3d"],
  ["Severe Thunderstorm Warning", "#ffa500"],
  ["Flash Flood Warning", "#00c853"],
  ["Flood Warning", "#2e8b57"],
  ["Special Marine Warning", "#ff8c00"],
  ["Extreme Wind Warning", "#ff00ff"],
  ["Snow Squall Warning", "#c71585"],
  ["Dust Storm Warning", "#c8a064"],
  ["Flood Advisory", "#00ff7f"],
  ["Special Weather Statement", "#f5deb3"],
];
export const warningColor = (event: string) => WARNING_COLORS.find(([e]) => e === event)?.[1] ?? "#9aa4be";
const colorExpr = ["match", ["get", "event"], ...WARNING_COLORS.flat(), "#9aa4be"] as never;

export interface WarningDetails {
  id: string;
  event: string;
  areaDesc?: string;
  ends?: string;
  severity?: string;
}

export function WarningsOverlay({ onSelect }: { onSelect?: (w: WarningDetails) => void }) {
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const focused = useIsFocused();
  const { data: local } = useAlerts();
  const { data } = useQuery({
    queryKey: ["map-alerts", serverUrl],
    queryFn: ({ signal }) => fetchMapAlerts(serverUrl, signal),
    enabled: extrasVisible && focused,
    refetchInterval: 60_000,
    staleTime: 60_000,
  });

  const geojson = useMemo<GeoJSON.FeatureCollection>(() => {
    if (!extrasVisible || !data?.features?.length) return EMPTY_FEATURE_COLLECTION;
    const drawn = new Set((local?.features ?? []).map((f) => f.properties.id));
    return { type: "FeatureCollection", features: data.features.filter((f) => !drawn.has(String(f.properties?.id))) };
  }, [data, local, extrasVisible]);

  return (
    <GeoJSONSource
      id="warnings-src"
      data={geojson}
      onPress={(event) => {
        const p = event.nativeEvent.features[0]?.properties;
        if (!p?.id || !p.event) return;
        event.stopPropagation();
        onSelect?.({ id: String(p.id), event: String(p.event), areaDesc: p.areaDesc ?? undefined, ends: p.ends ?? p.expires ?? undefined, severity: p.severity ?? undefined });
      }}
    >
      <Layer type="fill" id="warnings-fill" paint={{ "fill-color": colorExpr, "fill-opacity": 0.14 }} />
      <Layer type="line" id="warnings-line" paint={{ "line-color": colorExpr, "line-width": ["interpolate", ["linear"], ["zoom"], 4, 1.5, 9, 3] as never }} />
    </GeoJSONSource>
  );
}
