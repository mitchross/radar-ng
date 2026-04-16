import MapLibreGL, { type MapViewRef } from "@maplibre/maplibre-react-native";
import { useRef } from "react";
import { StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS, resolveMapStyleUrl } from "../../lib/constants";

MapLibreGL.setAccessToken(null);

interface WeatherMapProps {
  children?: React.ReactNode;
}

export function WeatherMap({ children }: WeatherMapProps) {
  const mapRef = useRef<MapViewRef>(null);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const dataSource = useWeatherStore((s) => s.dataSource);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  const centerCoord: [number, number] = [
    longitude ?? DEFAULTS.LONGITUDE,
    latitude ?? DEFAULTS.LATITUDE,
  ];

  const mapStyleUrl = resolveMapStyleUrl(dataSource, serverUrl, mapStyle);

  return (
    <MapLibreGL.MapView
      ref={mapRef}
      style={styles.map}
      mapStyle={mapStyleUrl}
      logoEnabled={false}
      attributionEnabled={true}
      attributionPosition={{ bottom: 8, left: 8 }}
    >
      <MapLibreGL.Camera
        defaultSettings={{
          centerCoordinate: centerCoord,
          zoomLevel: latitude ? 7 : DEFAULTS.ZOOM,
        }}
      />
      <MapLibreGL.UserLocation visible={true} />
      {children}
    </MapLibreGL.MapView>
  );
}

const styles = StyleSheet.create({
  map: {
    flex: 1,
  },
});
