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
import MapLibreGL from "@maplibre/maplibre-react-native";
import { useAlerts } from "../../hooks/useAlerts";

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

export function AlertPolygon() {
  const { data: alertData } = useAlerts();

  if (!alertData || alertData.features.length === 0) return null;

  const alertsWithGeometry = alertData.features.filter((f) => f.geometry !== null);
  if (alertsWithGeometry.length === 0) return null;

  const geojson: GeoJSON.FeatureCollection = {
    type: "FeatureCollection",
    features: alertsWithGeometry.map((alert) => ({
      type: "Feature" as const,
      geometry: alert.geometry!,
      properties: {
        severity: alert.properties.severity,
        event: alert.properties.event,
        kind: classifyEvent(alert.properties.event),
      },
    })),
  };

  const fillOpacityByKind = [
    "match",
    ["get", "kind"],
    "warning", 1.0,
    "watch", 0.65,
    "advisory", 0.4,
    0.25,
  ] as never;

  return (
    <MapLibreGL.ShapeSource id="alert-polygons" shape={geojson}>
      {/* Fill — opacity scales by alert kind so watches/advisories recede. */}
      <MapLibreGL.FillLayer
        id="alert-fill"
        style={{
          fillColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_FILL.Extreme,
            "Severe", SEVERITY_FILL.Severe,
            "Moderate", SEVERITY_FILL.Moderate,
            "Minor", SEVERITY_FILL.Minor,
            SEVERITY_FILL.Unknown,
          ],
          fillOpacity: fillOpacityByKind,
        }}
      />
      {/* Solid outline for warnings (active hazards). */}
      <MapLibreGL.LineLayer
        id="alert-outline-warning"
        filter={["==", ["get", "kind"], "warning"] as never}
        style={{
          lineColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          lineWidth: 2.5,
        }}
      />
      {/* Dashed outline for watches. */}
      <MapLibreGL.LineLayer
        id="alert-outline-watch"
        filter={["==", ["get", "kind"], "watch"] as never}
        style={{
          lineColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          lineWidth: 2,
          lineDasharray: [3, 3] as never,
        }}
      />
      {/* Dotted outline for advisories + statements. */}
      <MapLibreGL.LineLayer
        id="alert-outline-advisory"
        filter={["in", ["get", "kind"], ["literal", ["advisory", "statement"]]] as never}
        style={{
          lineColor: [
            "match",
            ["get", "severity"],
            "Extreme", SEVERITY_STROKE.Extreme,
            "Severe", SEVERITY_STROKE.Severe,
            "Moderate", SEVERITY_STROKE.Moderate,
            "Minor", SEVERITY_STROKE.Minor,
            SEVERITY_STROKE.Unknown,
          ],
          lineWidth: 1.5,
          lineDasharray: [1, 2] as never,
        }}
      />
    </MapLibreGL.ShapeSource>
  );
}
