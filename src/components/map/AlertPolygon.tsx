import MapLibreGL from "@maplibre/maplibre-react-native";
import { useAlerts } from "../../hooks/useAlerts";

const SEVERITY_FILL: Record<string, string> = {
  Extreme: "rgba(211, 47, 47, 0.25)",
  Severe: "rgba(244, 67, 54, 0.2)",
  Moderate: "rgba(255, 152, 0, 0.2)",
  Minor: "rgba(255, 193, 7, 0.15)",
  Unknown: "rgba(158, 158, 158, 0.1)",
};

const SEVERITY_STROKE: Record<string, string> = {
  Extreme: "#d32f2f",
  Severe: "#f44336",
  Moderate: "#ff9800",
  Minor: "#ffc107",
  Unknown: "#9e9e9e",
};

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
      },
    })),
  };

  return (
    <MapLibreGL.ShapeSource id="alert-polygons" shape={geojson}>
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
        }}
      />
      <MapLibreGL.LineLayer
        id="alert-outline"
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
        }}
      />
    </MapLibreGL.ShapeSource>
  );
}
