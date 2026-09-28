/**
 * Cumulus radar screen — full-bleed MapLibre with Apple-Weather polish.
 *
 *   • LayerLegendCard  (top-left, layer-aware vertical scale)
 *   • LayerLocationMarker  (user location pill w/ live layer value + tail)
 *   • Close button     (the tab bar is hidden on this route)
 *   • RadarFABs        (map options + locate capsule)
 *   • MapOptionsSheet  (layer, map style, storm overlays)
 *   • EyedropperPin    (long-press map → readout)
 *   • TimelineBar      ("Reflectivity / Sunday, April 19 2026" header)
 */
import { useState } from "react";
import { View, StyleSheet, Text, Pressable } from "react-native";
import { useIsFocused, useRouter } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { SymbolView } from "expo-symbols";
import { MapChromeSurface } from "../../components/ui/MapChromeSurface";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { LightningOverlay } from "../../components/map/LightningOverlay";
import {
  TropicalOverlay,
  type TropicalStormDetails,
} from "../../components/map/TropicalOverlay";
import { TropicalDetailSheet } from "../../components/map/TropicalDetailSheet";
import { StormCellsOverlay } from "../../components/map/StormCellsOverlay";
import { WindParticlesOverlay, useSharedCamera } from "../../components/map/WindParticlesOverlay";
import { DEFAULTS, MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { LayerLegendCard } from "../../components/map/LayerLegendCard";
import { LayerLocationMarker } from "../../components/map/LayerLocationMarker";
import { TimelineBar } from "../../components/timeline/TimelineBar";
import { RadarFABs } from "../../components/map/RadarFABs";
import { MapOptionsSheet } from "../../components/map/MapOptionsSheet";
import { EyedropperPin, InspectorPanel, useInspectReading, type PinnedPoint } from "../../components/inspector/Eyedropper";
import { useManifest } from "../../hooks/useManifest";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { useAlerts } from "../../hooks/useAlerts";
import { useWeatherStore } from "../../stores/useWeatherStore";

export default function RadarScreen() {
  useManifest();
  const { alertStatus } = useAlerts();
  const router = useRouter();

  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const focused = useIsFocused();
  const chrome = useMapChromeInsets();

  const [pinned, setPinned] = useState<PinnedPoint | null>(null);
  const inspect = useInspectReading(pinned);
  const [selectedTropical, setSelectedTropical] = useState<TropicalStormDetails | null>(null);
  const [mapOptionsOpen, setMapOptionsOpen] = useState(false);

  const camera = useSharedCamera(DEFAULTS.LONGITUDE, DEFAULTS.LATITUDE, DEFAULTS.ZOOM);
  // Particles also run over the air-quality heatmap (the IQAir Earth look):
  // the wind field is what carries the smoke plume, so seeing it flow over
  // the PM2.5 wash explains where the plume is headed.
  const windParticlesOn = activeLayer === "wind" || activeLayer === "air-quality";

  return (
    <View style={styles.container}>
      {/* Status text follows the basemap, not the app theme; only while this tab shows. */}
      {focused ? <StatusBar style={mapStyle === "light" ? "dark" : "light"} /> : null}
      <WeatherMap
        onLongPress={(lat, lon) => {
          // Readings load only while paused, so pinning a point stops the loop.
          setIsPlaying(false);
          setPinned({ lat, lon });
        }}
        trackCameraContinuously={windParticlesOn}
        onCameraChanged={(c) => {
          camera.lon.set(c.lon);
          camera.lat.set(c.lat);
          camera.zoom.set(c.zoom);
        }}
      >
        {(activeLayer === "radar" || activeLayer === "radar-hrrr") && <RadarOverlay />}
        {activeLayer === "temperature" && <WeatherLayerOverlay layerId="temperature" opacity={radarOpacity} />}
        {activeLayer === "precip-type" && <WeatherLayerOverlay layerId="precip-type" opacity={radarOpacity} />}
        {activeLayer === "wind" && <WeatherLayerOverlay layerId="wind" opacity={0.6} />}
        {activeLayer === "cape" && <WeatherLayerOverlay layerId="cape" opacity={0.5} />}
        {activeLayer === "precip-accum" && <WeatherLayerOverlay layerId="precip-accum" opacity={radarOpacity} />}
        {activeLayer === "cloud" && <WeatherLayerOverlay layerId="cloud" opacity={0.65} />}
        {activeLayer === "air-quality" && <WeatherLayerOverlay layerId="air-quality" opacity={0.75} />}
        {activeLayer === "ozone" && <WeatherLayerOverlay layerId="ozone" opacity={0.75} />}
        {/* Always mounted: each overlay renders an empty collection / hidden pin when
            it has nothing to show, so the map's native child count never churns. */}
        <AlertPolygon />
        <TropicalOverlay onSelect={setSelectedTropical} />
        <StormCellsOverlay />
        <LightningOverlay />
        <LayerLocationMarker inspecting={pinned != null} />
        <EyedropperPin pinned={pinned} readout={inspect.readout} />
      </WeatherMap>

      {/* The tab bar is hidden on this full-screen route, so this is the way back. */}
      <MapChromeSurface
        style={[styles.close, { top: chrome.top, left: chrome.left }]}
        fallbackStyle={styles.closeFill}
        colorScheme="light"
        interactive
      >
        <Pressable
          onPress={() => router.navigate("/")}
          style={({ pressed }) => [styles.closeButton, pressed ? { opacity: 0.55 } : null]}
          accessibilityRole="button"
          accessibilityLabel="Close radar"
        >
          <SymbolView name={{ ios: "xmark", android: "close" }} size={17} tintColor="#1a2030" weight="bold" />
        </Pressable>
      </MapChromeSurface>

      {/* Only when alerts are stale or failing; left of the map controls. */}
      {alertStatus.kind !== "current" ? (
        <Text maxFontSizeMultiplier={MAP_CHROME_MAX_FONT_SCALE}
          accessibilityRole="alert"
          accessibilityLabel={alertStatus.accessibilityLabel}
          pointerEvents="none"
          style={[styles.alertStatus, { top: chrome.top, right: chrome.right + 58 }]}
        >
          ALERTS {alertStatus.label}
        </Text>
      ) : null}

      {/* Wind particles — Skia canvas overlay, active on the wind layer */}
      <WindParticlesOverlay enabled={windParticlesOn} camera={camera} />

      {/* Vertical legend card (top-left) */}
      <LayerLegendCard activeLayer={activeLayer} />

      {/* Readout for a long-pressed point: above the map chrome, clear of legend and buttons. */}
      <InspectorPanel pinned={pinned} inspect={inspect} onClear={() => setPinned(null)} />

      {/* Map options + locate. The inspector panel's own button clears a pin. */}
      <RadarFABs onOpenMapOptions={() => setMapOptionsOpen(true)} />

      <MapOptionsSheet visible={mapOptionsOpen} onClose={() => setMapOptionsOpen(false)} />

      {/* Timeline — past observed + nowcast + HRRR forecast in one stream */}
      <TimelineBar />
      <TropicalDetailSheet
        storm={selectedTropical}
        onClose={() => setSelectedTropical(null)}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: "#0a0e1a" },
  close: { position: "absolute", zIndex: 15, width: 48, height: 48, borderRadius: 24, overflow: "hidden" },
  closeFill: {
    backgroundColor: "rgba(255,255,255,0.92)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(10,20,40,0.12)",
    shadowColor: "#000",
    shadowOpacity: 0.14,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
  closeButton: { width: 48, height: 48, alignItems: "center", justifyContent: "center" },
  alertStatus: {
    position: "absolute",
    zIndex: 15,
    minHeight: 44,
    paddingHorizontal: 12,
    lineHeight: 44,
    textAlignVertical: "center",
    borderRadius: 12,
    overflow: "hidden",
    color: "#FFFFFF",
    backgroundColor: "rgba(10,10,20,0.82)",
    fontSize: 10,
    fontWeight: "800",
    letterSpacing: 0.8,
  },
});
