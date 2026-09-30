/**
 * Tropical cyclone overlay — three layered styles keyed by `kind`:
 *   cone     → soft red translucent polygon (forecast uncertainty)
 *   track    → dashed red linestring (forecast positions)
 *   position → red pulsing symbol + storm name label
 *   forecast_point → NHC forecast positions coloured by Saffir-Simpson category
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useTropical } from "../../hooks/useTropical";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export interface TropicalStormDetails {
  stormId: string;
  name: string;
  classification?: string;
  windMph?: number;
  pressureMb?: number;
  updatedAt?: string;
  category?: number;
  movementMph?: number;
  movementDirDeg?: number;
}

const CATEGORY_COLOR = ["step", ["coalesce", ["get", "category"], 0], "#4aa3ff", 1, "#ffd23f", 2, "#ff9f1c", 3, "#ff4d4d", 4, "#d61f69", 5, "#9b30ff"] as never;

export function TropicalOverlay({
  onSelect,
}: {
  onSelect?: (storm: TropicalStormDetails) => void;
}) {
  const { data } = useTropical();
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  // Label glyphs must come from the active basemap's own glyph server.
  const { labelFont } = useBasemapStyle(serverUrl, mapStyle);
  // Always mounted (empty collection off-season) — see lib/emptyGeoJSON.
  const geojson = data && data.features.length > 0
    ? (data as GeoJSON.FeatureCollection)
    : EMPTY_FEATURE_COLLECTION;

  return (
    <GeoJSONSource
      id="tropical-src"
      data={geojson}
      hitbox={{ top: 28, right: 28, bottom: 28, left: 28 }}
      onPress={(event) => {
        event.stopPropagation();
        const position = event.nativeEvent.features.find(
          (feature) => feature.properties?.kind === "position",
        );
        const storm = position ? stormDetails(position.properties) : null;
        if (storm) onSelect?.(storm);
      }}
    >
      {/* Cone (under everything else) */}
      <Layer
        type="fill"
        id="tropical-cone"
        filter={["==", ["get", "kind"], "cone"] as never}
        paint={{
          "fill-color": "#FF3B4A",
          "fill-opacity": 0.15,
          "fill-outline-color": "#FF3B4A",
        }}
      />
      {/* Forecast track line */}
      <Layer
        type="line"
        id="tropical-track"
        filter={["==", ["get", "kind"], "track"] as never}
        paint={{
          "line-color": "#FF3B4A",
          "line-width": 2,
          "line-dasharray": [2, 2] as never,
          "line-opacity": 0.9,
        }}
      />
      {/* NHC forecast points, coloured by category, labelled with it */}
      <Layer
        type="circle"
        id="tropical-forecast-points"
        filter={["==", ["get", "kind"], "forecast_point"] as never}
        paint={{ "circle-radius": 7, "circle-color": CATEGORY_COLOR, "circle-stroke-color": "#111827", "circle-stroke-width": 1.5 }}
      />
      <Layer
        type="symbol"
        id="tropical-forecast-labels"
        filter={["==", ["get", "kind"], "forecast_point"] as never}
        layout={{
          "text-field": ["coalesce", ["to-string", ["get", "category"]], ["get", "storm_type"], ""] as never,
          "text-font": labelFont,
          "text-size": 10,
          "text-allow-overlap": true,
        }}
        paint={{ "text-color": "#111827" }}
      />
      {/* Current storm position — circle + stroke */}
      <Layer
        type="circle"
        id="tropical-position"
        filter={["==", ["get", "kind"], "position"] as never}
        paint={{
          "circle-radius": 9,
          "circle-color": ["case", ["to-boolean", ["get", "category"]], CATEGORY_COLOR, "#FF3B4A"] as never,
          "circle-stroke-color": "#FFFFFF",
          "circle-stroke-width": 2.5,
          "circle-opacity": 0.95,
        }}
      />
      {/* A bare red dot looks like an unexplained map artifact. Name the
          active NHC storm and include its classification beside the fix. */}
      <Layer
        type="symbol"
        id="tropical-position-label"
        filter={["==", ["get", "kind"], "position"] as never}
        layout={{
          "text-field": [
            "concat",
            ["get", "name"],
            " · ",
            ["case", ["to-boolean", ["get", "category"]], ["concat", "Cat ", ["to-string", ["get", "category"]]], ["coalesce", ["get", "classification"], "Storm"]],
          ] as never,
          "text-size": 12,
          "text-font": labelFont,
          "text-offset": [0, 1.6],
          "text-anchor": "top",
          "text-allow-overlap": true,
        }}
        paint={{
          "text-color": "#9F1422",
          "text-halo-color": "#FFFFFF",
          "text-halo-width": 1.5,
        }}
      />
    </GeoJSONSource>
  );
}

function stormDetails(properties: GeoJSON.GeoJsonProperties): TropicalStormDetails | null {
  if (!properties) return null;
  const name = stringValue(properties.name);
  const stormId = stringValue(properties.storm_id);
  if (!name || !stormId) return null;
  return {
    name,
    stormId,
    classification: stringValue(properties.classification),
    windMph: numberValue(properties.wind_mph),
    pressureMb: numberValue(properties.pressure_mb),
    updatedAt: stringValue(properties.updated_at),
    category: numberValue(properties.category),
    movementMph: numberValue(properties.movement_mph),
    movementDirDeg: numberValue(properties.movement_dir_deg),
  };
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}
