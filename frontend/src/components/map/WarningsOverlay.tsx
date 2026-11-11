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
import { useReplayWarningsQuery } from "../../hooks/useReplay";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { warningTagExpression } from "../../lib/warningTags";
import type { PinnedPoint } from "../inspector/Eyedropper";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";
import { replayWarningCollection, replayWarningCopy, replayWarningKey, type ReplayWarningCopy, type ReplayWarningProps } from "../../lib/replaySource";

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
  /** Set for a replayed warning: popup text in the case's zone; no live explanation or outlook. */
  historical?: ReplayWarningCopy;
}

export function WarningsOverlay({ onSelect }: { onSelect?: (w: WarningDetails, point?: PinnedPoint) => void }) {
  const replaying = useWeatherStore((s) => s.source.kind === "replay");
  const timeZone = useWeatherStore((s) => s.replayCase?.timezone ?? "UTC");
  // Playhead in ms; null outside replay so live ticks don't re-render this overlay.
  const playheadMs = useWeatherStore((s) => (s.source.kind === "replay" ? (s.frames[s.currentFrameIndex]?.time ?? 0) * 1000 : null));
  const extrasVisible = useWeatherStore((s) => s.extrasVisible && s.source.kind === "live");
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const { labelFont } = useBasemapStyle(serverUrl, mapStyle);
  const focused = useIsFocused();
  const { data: local } = useAlerts();
  const { data } = useQuery({
    queryKey: ["map-alerts", serverUrl],
    queryFn: ({ signal }) => fetchMapAlerts(serverUrl, signal),
    enabled: extrasVisible && focused,
    refetchInterval: 60_000,
    staleTime: 60_000,
  });
  const { data: history } = useReplayWarningsQuery();

  // Versions in force at the playhead; the same key reuses the object so the source redraws only on change.
  const replayFc = replaying ? replayWarningCollection(history ?? null, playheadMs ?? NaN) : null;
  const replayKey = replayFc ? replayWarningKey(replayFc) : null;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const replayGeo = useMemo(() => replayFc as unknown as GeoJSON.FeatureCollection | null, [replayKey, history]);

  const liveGeo = useMemo<GeoJSON.FeatureCollection>(() => {
    if (!extrasVisible || !data?.features?.length) return EMPTY_FEATURE_COLLECTION;
    const drawn = new Set((local?.features ?? []).map((f) => f.properties.id));
    return { type: "FeatureCollection", features: data.features.filter((f) => !drawn.has(String(f.properties?.id))) };
  }, [data, local, extrasVisible]);
  const geojson = replaying ? (replayGeo?.features.length ? replayGeo : EMPTY_FEATURE_COLLECTION) : liveGeo;

  return (
    <GeoJSONSource
      id="warnings-src"
      data={geojson}
      onPress={(event) => {
        const p = event.nativeEvent.features[0]?.properties;
        if (!p?.id || !p.event) return;
        event.stopPropagation();
        const warning: WarningDetails = { id: String(p.id), event: String(p.event), areaDesc: p.areaDesc ?? undefined, ends: p.ends ?? p.expires ?? undefined, severity: p.severity ?? undefined };
        if (replaying) warning.historical = replayWarningCopy(p as Partial<ReplayWarningProps>, timeZone);
        const coordinates = event.nativeEvent.lngLat;
        if (coordinates) onSelect?.(warning, { lon: coordinates[0], lat: coordinates[1] });
        else onSelect?.(warning);
      }}
    >
      <Layer type="fill" id="warnings-fill" paint={{ "fill-color": colorExpr, "fill-opacity": 0.14 }} />
      <Layer type="line" id="warnings-line" paint={{ "line-color": colorExpr, "line-width": ["interpolate", ["linear"], ["zoom"], 4, 1.5, 9, 3] as never }} />
      {/* Type tag (TOR/SVR/FFW...) so colour is not the only signal; minzoom keeps the zoomed-out map clean. */}
      <Layer
        type="symbol"
        id="warnings-labels"
        minzoom={5.5}
        layout={{ "text-field": warningTagExpression() as never, "text-font": labelFont, "text-size": 12, "text-padding": 4 }}
        paint={{ "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 }}
      />
    </GeoJSONSource>
  );
}
