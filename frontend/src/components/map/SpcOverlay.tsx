import { GeoJSONSource, Layer } from "@maplibre/maplibre-react-native";
import { useQuery } from "@tanstack/react-query";
import { useIsFocused } from "expo-router";
import { fetchSpc } from "../../lib/api";
import { activeSpc, spcColorExpression, spcProduct, spcTagExpression, type SpcKind, type SpcProduct } from "../../lib/spc";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { useSpcNow } from "../../hooks/useSpcNow";
import { EMPTY_FEATURE_COLLECTION } from "../../lib/emptyGeoJSON";

export interface SpcSelection { product: SpcProduct; lat: number; lon: number }

export function SpcOverlay({ kind, onSelect }: { kind: SpcKind; onSelect: (selection: SpcSelection) => void }) {
  const enabled = useWeatherStore((s) => s.source.kind === "live" && (kind === "watches" ? s.spcWatchesVisible : s.spcDiscussionsVisible));
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const { labelFont } = useBasemapStyle(serverUrl, mapStyle);
  const focused = useIsFocused();
  const now = useSpcNow(enabled && focused);
  const { data } = useQuery({
    queryKey: ["spc-products", serverUrl, kind],
    queryFn: ({ signal }) => fetchSpc(serverUrl, kind, signal),
    enabled: enabled && focused,
    staleTime: 60_000,
    refetchInterval: 60_000,
  });
  const geojson = enabled && data ? activeSpc(data, now) : EMPTY_FEATURE_COLLECTION;
  return <GeoJSONSource id={`spc-${kind}`} data={geojson} onPress={(event) => {
    const product = spcProduct(event.nativeEvent.features[0]?.properties, Date.now());
    if (!product) return;
    event.stopPropagation();
    const [lon, lat] = event.nativeEvent.lngLat;
    onSelect({ product, lon, lat });
  }}>
    <Layer id={`spc-${kind}-hit`} type="fill" paint={{ "fill-opacity": 0 }} />
    <Layer id={`spc-${kind}-line`} type="line" paint={{ "line-color": spcColorExpression() as never, "line-width": 2.5 }} />
    <Layer id={`spc-${kind}-labels`} type="symbol" layout={{ "text-field": spcTagExpression() as never, "text-font": labelFont, "text-size": 12, "text-padding": 4 }}
      paint={{ "text-color": "#ffffff", "text-halo-color": "#111827", "text-halo-width": 1.6 }} />
  </GeoJSONSource>;
}
