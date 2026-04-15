import { useRef } from "react";
import MapLibreGL from "@maplibre/maplibre-react-native";
import { StyleSheet } from "react-native";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { MAP_STYLES, DEFAULTS } from "../../lib/constants";

MapLibreGL.setAccessToken(null);

interface WeatherMapProps {
  children?: React.ReactNode;
}

export function WeatherMap({ children }: WeatherMapProps) {
  const mapRef = useRef<MapLibreGL.MapView>(null);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);

  const centerCoord: [number, number] = [
    longitude ?? DEFAULTS.LONGITUDE,
    latitude ?? DEFAULTS.LATITUDE,
  ];

  return (
    <MapLibreGL.MapView
      ref={mapRef}
      style={styles.map}
      styleURL={MAP_STYLES[mapStyle]}
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
