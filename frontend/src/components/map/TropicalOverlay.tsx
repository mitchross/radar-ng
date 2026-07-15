/**
 * Tropical cyclone overlay — three layered styles keyed by `kind`:
 *   cone     → soft red translucent polygon (forecast uncertainty)
 *   track    → dashed red linestring (forecast positions)
 *   position → red pulsing symbol + storm name label
 */
import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useTropical } from "../../hooks/useTropical";

export interface TropicalStormDetails {
  stormId: string;
  name: string;
  classification?: string;
  windMph?: number;
  pressureMb?: number;
  updatedAt?: string;
}

export function TropicalOverlay({
  onSelect,
}: {
  onSelect?: (storm: TropicalStormDetails) => void;
}) {
  const { data } = useTropical();
  if (!data || data.features.length === 0) return null;

  return (
    <GeoJSONSource
      id="tropical-src"
      data={data as GeoJSON.FeatureCollection}
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
      {/* Current storm position — circle + stroke */}
      <Layer
        type="circle"
        id="tropical-position"
        filter={["==", ["get", "kind"], "position"] as never}
        paint={{
          "circle-radius": 9,
          "circle-color": "#FF3B4A",
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
            ["coalesce", ["get", "classification"], "Storm"],
          ] as never,
          "text-size": 12,
          "text-font": ["Noto Sans Regular"],
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
  };
}

function stringValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function numberValue(value: unknown): number | undefined {
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}
