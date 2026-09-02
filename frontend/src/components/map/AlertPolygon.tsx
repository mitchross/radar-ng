/**
 * NWS alert polygons with per-type styling.
 *
 * Event classification (from the alert's `event` text):
 *   warning  — active hazard, solid outline + filled fill
 *   watch    — conditions are favorable, dashed outline + lighter fill
 *   advisory — less severe, dotted outline + very light fill
 *
 * Severity still drives color (Extreme/Severe/Moderate/Minor).
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useMemo } from "react";
import { useAlerts } from "../../hooks/useAlerts";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

const SEVERITY_FILL: Record<string, string> = {
  Extreme: "rgba(211, 47, 47, 0.30)",
  Severe: "rgba(244, 67, 54, 0.22)",
  Moderate: "rgba(255, 152, 0, 0.22)",
  Minor: "rgba(255, 193, 7, 0.18)",
  Unknown: "rgba(158, 158, 158, 0.12)",
};

const SEVERITY_STROKE: Record<string, string> = {
  Extreme: "#d32f2f",
  Severe: "#f44336",
  Moderate: "#ff9800",
  Minor: "#ffc107",
  Unknown: "#9e9e9e",
};

function classifyEvent(event: string): "warning" | "watch" | "advisory" | "statement" {
  const e = event.toLowerCase();
  if (e.includes("warning")) return "warning";
  if (e.includes("watch")) return "watch";
  if (e.includes("advisory")) return "advisory";
  return "statement";
}

// Always mounted (empty collection when there are no alerts) — see lib/emptyGeoJSON.
export function AlertPolygon() {
  const { data: alertData } = useAlerts();

  const geojson = useMemo<GeoJSON.FeatureCollection>(() => {
    const withGeometry = alertData?.features.filter((f) => f.geometry !== null) ?? [];
    if (withGeometry.length === 0) return EMPTY_FEATURE_COLLECTION;
    return {
      type: "FeatureCollection",
      features: withGeometry.map((alert) => ({
        type: "Feature" as const,
        geometry: alert.geometry!,
        properties: {
          severity: alert.properties.severity,
          event: alert.properties.event,
          kind: classifyEvent(alert.properties.event),
        },
      })),
    };
  }, [alertData]);

  const fillOpacityByKind = [
    "match",
    ["get", "kind"],
    "warning", 1.0,
    "watch", 0.65,
    "advisory", 0.4,
    0.25,
  ] as never;

  return (
    <GeoJSONSource id="alert-polygons" data={geojson}>
      {/* Fill — opacity scales by alert kind so watches/advisories recede. */}
      <Layer
        type="fill"
        id="alert-fill"
        paint={{
          "fill-color": [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_FILL.Extreme,
            "Severe", SEVERITY_FILL.Severe,
            "Moderate", SEVERITY_FILL.Moderate,
            "Minor", SEVERITY_FILL.Minor,
            SEVERITY_FILL.Unknown,
          ],
          "fill-opacity": fillOpacityByKind,
        }}
      />
      {/* Solid outline for warnings (active hazards). */}
      <Layer
        type="line"
        id="alert-outline-warning"
        filter={["==", ["get", "kind"], "warning"] as never}
        paint={{
          "line-color": [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          "line-width": 2.5,
        }}
      />
      {/* Dashed outline for watches. */}
      <Layer
        type="line"
        id="alert-outline-watch"
        filter={["==", ["get", "kind"], "watch"] as never}
        paint={{
          "line-color": [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          "line-width": 2,
          "line-dasharray": [3, 3] as never,
        }}
      />
      {/* Dotted outline for advisories + statements. */}
      <Layer
        type="line"
        id="alert-outline-advisory"
        filter={["in", ["get", "kind"], ["literal", ["advisory", "statement"]]] as never}
        paint={{
          "line-color": [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          "line-width": 1.5,
          "line-dasharray": [1, 2] as never,
        }}
      />
    </GeoJSONSource>
  );
}
