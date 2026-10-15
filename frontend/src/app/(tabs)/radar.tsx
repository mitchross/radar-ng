/**
 * Cumulus radar screen — full-bleed MapLibre with Apple-Weather polish.
 *
 *   • LayerLegendCard  (top-left, layer-aware vertical scale)
 *   • LayerLocationMarker  (user location pill w/ live layer value + tail)
 *   • Close button     (the tab bar is hidden on this route)
 *   • RadarStatusChip  (LIVE / UPDATED 12M AGO / OFFLINE, same as Home)
 *   • RadarStatusNotice (loading, offline or unavailable card)
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
import { WarningsOverlay, type WarningDetails } from "../../components/map/WarningsOverlay";
import { OutlookOverlay } from "../../components/map/OutlookOverlay";
import { StormCellSheet, WarningSheet } from "../../components/map/FeatureSheets";
import type { StormCellInfo } from "../../lib/stormTracks";
import { WindParticlesOverlay, useSharedCamera } from "../../components/map/WindParticlesOverlay";
import { PackedRadarOverlay } from "../../components/map/PackedRadarOverlay";
import { DEFAULTS, MAP_CHROME_MAX_FONT_SCALE } from "../../lib/constants";
import { LayerLegendCard } from "../../components/map/LayerLegendCard";
import { LayerLocationMarker } from "../../components/map/LayerLocationMarker";
import { TimelineBar } from "../../components/timeline/TimelineBar";
import { RadarFABs } from "../../components/map/RadarFABs";
import { RadarStatusChip } from "../../components/map/RadarStatusChip";
import { RadarStatusNotice } from "../../components/map/RadarStatusNotice";
import { MapOptionsSheet } from "../../components/map/MapOptionsSheet";
import { EyedropperPin, InspectorPanel, useInspectReading, type PinnedPoint } from "../../components/inspector/Eyedropper";
import { useManifest } from "../../hooks/useManifest";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { useAlerts } from "../../hooks/useAlerts";
import { useRadarStatus } from "../../hooks/useRadarStatus";
import { usePackedFrames } from "../../hooks/usePackedFrames";
import { useWeatherStore } from "../../stores/useWeatherStore";
import type { LayerType } from "../../types/weather";

/** Model layers drawn by WeatherLayerOverlay (rasters) or PackedRadarOverlay (packed frames). */
const WEATHER_LAYERS: readonly LayerType[] = [
  "temperature",
  "feels-like",
  "dewpoint",
  "precip-type",
  "wind",
  "cape",
  "precip-accum",
  "cloud",
  "air-quality",
  "ozone",
];
/** Layers with a fixed opacity; the rest follow the radar opacity setting. */
const FIXED_LAYER_OPACITY: Partial<Record<LayerType, number>> = {
  wind: 0.6,
  cape: 0.5,
  cloud: 0.65,
  "air-quality": 0.75,
  ozone: 0.75,
};

export default function RadarScreen() {
  useManifest();
  const { alertStatus } = useAlerts();
  const radar = useRadarStatus();
  const hasFrames = useWeatherStore((s) => s.frames.length > 0);
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
  const [selectedCell, setSelectedCell] = useState<StormCellInfo | null>(null);
  const [selectedWarning, setSelectedWarning] = useState<WarningDetails | null>(null);
  const [mapOptionsOpen, setMapOptionsOpen] = useState(false);

  // Same start as WeatherMap's initial view, so overlays line up before the first region event.
  const camera = useSharedCamera(
    useWeatherStore.getState().longitude ?? DEFAULTS.LONGITUDE,
    useWeatherStore.getState().latitude ?? DEFAULTS.LATITUDE,
    DEFAULTS.ZOOM,
  );
  // Radar frames with packed tiles draw on the GPU canvas; the rest stay MapLibre rasters.
  const packed = usePackedFrames();
  const layerOpacity = FIXED_LAYER_OPACITY[activeLayer] ?? radarOpacity;
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
        trackCameraContinuously={windParticlesOn || packed != null}
        allowPitch={packed == null}
        onCameraChanged={(c) => {
          camera.lon.set(c.lon);
          camera.lat.set(c.lat);
          camera.zoom.set(c.zoom);
          camera.bearing.set(c.bearing);
        }}
      >
        {(activeLayer === "radar" || activeLayer === "radar-hrrr") && <RadarOverlay isPacked={packed?.isPacked} />}
        {WEATHER_LAYERS.includes(activeLayer) && (
          <WeatherLayerOverlay key={activeLayer} layerId={activeLayer} opacity={layerOpacity} isPacked={packed?.isPacked} />
        )}
        {/* Always mounted: each overlay renders an empty collection / hidden pin when
            it has nothing to show, so the map's native child count never churns. */}
        <OutlookOverlay />
        <WarningsOverlay onSelect={setSelectedWarning} />
        <AlertPolygon />
        <TropicalOverlay onSelect={setSelectedTropical} />
        <StormCellsOverlay onSelect={setSelectedCell} />
        <LightningOverlay />
        <LayerLocationMarker inspecting={pinned != null} />
        <EyedropperPin pinned={pinned} readout={inspect.readout} />
      </WeatherMap>
      {packed ? <PackedRadarOverlay packed={packed} camera={camera} opacity={layerOpacity} /> : null}

      {/* The tab bar is hidden on this full-screen route, so this is the way back. */}
      <MapChromeSurface
        style={[styles.close, { top: chrome.top, left: chrome.left }]}
        fallbackStyle={styles.closeFill}
        colorScheme="light"
        interactive
      >
        <Pressable
          onPress={() => (router.canGoBack() ? router.back() : router.navigate("/"))}
          style={({ pressed }) => [styles.closeButton, pressed ? { opacity: 0.55 } : null]}
          accessibilityRole="button"
          accessibilityLabel="Close radar"
        >
          <SymbolView name={{ ios: "xmark", android: "close" }} size={17} tintColor="#1a2030" weight="bold" />
        </Pressable>
      </MapChromeSurface>

      <RadarStatusChip status={radar.status} style={[styles.statusChip, { top: chrome.top + 14, left: chrome.left + 58 }]} />
      <RadarStatusNotice status={radar.status} hasFrames={hasFrames} onRetry={radar.refetch} top={chrome.belowClose} />

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
      <StormCellSheet cell={selectedCell} onClose={() => setSelectedCell(null)} />
      <WarningSheet warning={selectedWarning} onClose={() => setSelectedWarning(null)} />
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
  statusChip: { position: "absolute", zIndex: 15 },
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
