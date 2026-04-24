/**
 * Cumulus radar screen — full-bleed MapLibre with Apple-Weather polish.
 *
 *   • LayerLegendCard  (top-left, layer-aware vertical scale)
 *   • LayerLocationMarker  (user location pill w/ live layer value + tail)
 *   • RadarFABs        (right rail + layer-tinted popover)
 *   • MapStylePicker   (theme + projection, summoned by the rail)
 *   • EyedropperPin    (long-press map → readout)
 *   • TimelineBar      ("Reflectivity / Sunday, April 19 2026" header)
 */
import { useState } from "react";
import { View, StyleSheet, Text, Pressable } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useRouter } from "expo-router";
import { WeatherMap } from "../../components/map/WeatherMap";
import { RadarOverlay } from "../../components/map/RadarOverlay";
import { WeatherLayerOverlay } from "../../components/map/WeatherLayerOverlay";
import { AlertPolygon } from "../../components/map/AlertPolygon";
import { LightningOverlay } from "../../components/map/LightningOverlay";
import { TropicalOverlay } from "../../components/map/TropicalOverlay";
import { StormCellsOverlay } from "../../components/map/StormCellsOverlay";
import { WindParticlesOverlay, useSharedCamera } from "../../components/map/WindParticlesOverlay";
import { DEFAULTS } from "../../lib/constants";
import { LayerLegendCard } from "../../components/map/LayerLegendCard";
import { LayerLocationMarker } from "../../components/map/LayerLocationMarker";
import { TimelineBar } from "../../components/timeline/TimelineBar";
import { CurrentForecastToggle } from "../../components/timeline/CurrentForecastToggle";
import { RadarFABs } from "../../components/map/RadarFABs";
import { MapStylePicker } from "../../components/map/MapStylePicker";
import { EyedropperPin, type PinnedPoint } from "../../components/inspector/Eyedropper";
import { useManifest } from "../../hooks/useManifest";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { useAlerts } from "../../hooks/useAlerts";
import { cumulus } from "../../lib/cumulusTheme";

export default function RadarScreen() {
  useManifest();
  const router = useRouter();

  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const radarOpacity = useWeatherStore((s) => s.radarOpacity);
  const { data: alertData } = useAlerts();

  const [pinned, setPinned] = useState<PinnedPoint | null>(null);
  const [stylePickerOpen, setStylePickerOpen] = useState(false);

  const camera = useSharedCamera(DEFAULTS.LONGITUDE, DEFAULTS.LATITUDE, DEFAULTS.ZOOM);
  const windParticlesOn = activeLayer === "wind";

  const topAlert = alertData?.features?.[0];

  return (
    <View style={styles.container}>
      <WeatherMap
        onLongPress={(lat, lon) => setPinned({ lat, lon })}
        onCameraChanged={(c) => {
          camera.lon.value = c.lon;
          camera.lat.value = c.lat;
          camera.zoom.value = c.zoom;
        }}
      >
        {(activeLayer === "radar" || activeLayer === "radar-hrrr") && <RadarOverlay />}
        {activeLayer === "temperature" && <WeatherLayerOverlay layerId="temperature" opacity={radarOpacity} />}
        {activeLayer === "precip-type" && <WeatherLayerOverlay layerId="precip-type" opacity={radarOpacity} />}
        {activeLayer === "wind" && <WeatherLayerOverlay layerId="wind" opacity={0.6} />}
        {activeLayer === "cape" && <WeatherLayerOverlay layerId="cape" opacity={0.5} />}
        {activeLayer === "precip-accum" && <WeatherLayerOverlay layerId="precip-accum" opacity={radarOpacity} />}
        {activeLayer === "cloud" && <WeatherLayerOverlay layerId="cloud" opacity={0.65} />}
        <AlertPolygon />
        <TropicalOverlay />
        <StormCellsOverlay />
        <LightningOverlay />
        <LayerLocationMarker />
        {pinned && <EyedropperPin pinned={pinned} onClear={() => setPinned(null)} />}
      </WeatherMap>

      {/* Top safe area — close button + alert banner. */}
      <SafeAreaView style={styles.safeTop} edges={["top"]} pointerEvents="box-none">
        <Pressable
          style={styles.closeBtn}
          onPress={() => router.navigate("/" as never)}
          hitSlop={10}
          accessibilityRole="button"
          accessibilityLabel="Close radar"
        >
          <View style={[styles.closeLine, styles.closeLineA]} />
          <View style={[styles.closeLine, styles.closeLineB]} />
        </Pressable>
        {topAlert && (
          <View style={styles.alertBanner}>
            <View style={styles.alertDot} />
            <Text style={styles.alertText} numberOfLines={1}>
              {topAlert.properties.event}
            </Text>
          </View>
        )}
      </SafeAreaView>

      {/* Wind particles — Skia canvas overlay, active on the wind layer */}
      <WindParticlesOverlay enabled={windParticlesOn} camera={camera} />

      {/* Vertical legend card (top-left) */}
      <LayerLegendCard activeLayer={activeLayer} />

      {/* Right-side controls — crosshair button clears a pinned inspector if any. */}
      <RadarFABs
        inspectorActive={pinned != null}
        onToggleInspector={() => setPinned(null)}
        onOpenStylePicker={() => setStylePickerOpen(true)}
      />

      {/* Map style + projection picker */}
      <MapStylePicker visible={stylePickerOpen} onClose={() => setStylePickerOpen(false)} />

      {/* Current / Forecast segmented toggle (above timeline) */}
      <CurrentForecastToggle />

      {/* Timeline */}
      <TimelineBar />
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1, backgroundColor: "#0a0e1a" },
  safeTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    zIndex: 15,
  },
  alertBanner: {
    marginHorizontal: 12,
    marginTop: 48,
    paddingHorizontal: 12,
    paddingVertical: 7,
    borderRadius: 12,
    backgroundColor: "rgba(255,59,74,0.2)",
    borderWidth: 1,
    borderColor: "rgba(255,59,74,0.45)",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  alertDot: { width: 6, height: 6, borderRadius: 3, backgroundColor: cumulus.alert },
  alertText: { color: "#FF8A80", fontSize: 12, fontWeight: "600", flex: 1 },
  closeBtn: {
    alignSelf: "flex-start",
    marginLeft: 12,
    marginTop: 8,
    width: 36,
    height: 36,
    borderRadius: 18,
    backgroundColor: "rgba(10,10,20,0.78)",
    borderWidth: 1,
    borderColor: "rgba(255,255,255,0.12)",
    alignItems: "center",
    justifyContent: "center",
  },
  closeLine: {
    position: "absolute",
    width: 16,
    height: 2,
    borderRadius: 1,
    backgroundColor: "#FFFFFF",
  },
  closeLineA: { transform: [{ rotate: "45deg" }] },
  closeLineB: { transform: [{ rotate: "-45deg" }] },
});
