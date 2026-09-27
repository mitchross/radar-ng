import {
  Camera,
  Map,
  type CameraRef,
  type MapRef,
  type PressEvent,
  type ViewStateChangeEvent,
} from "@maplibre/maplibre-react-native";
import { Children, isValidElement, useEffect, useEffectEvent, useMemo, useRef } from "react";
import { Pressable, StyleSheet, View, type NativeSyntheticEvent } from "react-native";
import { SymbolView } from "expo-symbols";
import { useIsFocused } from "expo-router";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { DEFAULTS } from "../../lib/constants";
import { useBasemapStyle } from "../../hooks/useBasemapStyle";
import { useMapChromeInsets } from "../../hooks/useMapChromeInsets";
import { MapChromeSurface } from "../ui/MapChromeSurface";

const ZOOM_MIN = 1;
const ZOOM_MAX = 15;

interface WeatherMapProps {
  children?: React.ReactNode;
  onLongPress?: (lat: number, lon: number) => void;
  onCameraChanged?: (camera: { lon: number; lat: number; zoom: number }) => void;
  /**
   * Report every camera frame during a gesture, not just where it settles.
   * Only a live overlay (wind particles) needs this; each event crosses the bridge.
   */
  trackCameraContinuously?: boolean;
}

export function WeatherMap({
  children,
  onLongPress,
  onCameraChanged,
  trackCameraContinuously = false,
}: WeatherMapProps) {
  const mapRef = useRef<MapRef>(null);
  const chrome = useMapChromeInsets();
  const cameraRef = useRef<CameraRef>(null);
  const mapStyle = useWeatherStore((s) => s.mapStyle);
  const serverUrl = useWeatherStore((s) => s.serverUrl);
  const latitude = useWeatherStore((s) => s.latitude);
  const longitude = useWeatherStore((s) => s.longitude);
  const recenterNonce = useWeatherStore((s) => s.recenterNonce);
  const focusBounds = useWeatherStore((s) => s.focusBounds);
  const setFocusBounds = useWeatherStore((s) => s.setFocusBounds);
  const initialZoom = DEFAULTS.ZOOM;
  // Mirror current camera zoom so the +/- buttons can clamp without round-tripping.
  const zoomRef = useRef<number>(initialZoom);

  const { style: patchedStyle } = useBasemapStyle(serverUrl, mapStyle);

  const centerCoord = useMemo<[number, number]>(
    () => [
      longitude ?? DEFAULTS.LONGITUDE,
      latitude ?? DEFAULTS.LATITUDE,
    ],
    [latitude, longitude],
  );

  function zoomBy(delta: number) {
    const next = Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, zoomRef.current + delta));
    zoomRef.current = next;
    cameraRef.current?.setStop({ zoom: next, duration: 220 });
  }

  // Recenter only on intent (first fix, a new place, the Locate button), never
  // on GPS drift, and keep whatever zoom the user chose. Native tabs keep this
  // map mounted while hidden, and MapLibre drops camera moves on a hidden map,
  // so a request made elsewhere (choosing a city in Settings) waits until the
  // map is on screen.
  const focused = useIsFocused();
  const appliedRecenter = useRef(0);
  const recenter = useEffectEvent(() => {
    cameraRef.current?.setStop({ center: centerCoord, duration: 350 });
  });
  useEffect(() => {
    if (!focused || recenterNonce === appliedRecenter.current) return;
    appliedRecenter.current = recenterNonce;
    recenter();
  }, [recenterNonce, focused]);

  // Frame a requested area (e.g. "Open in Radar" from an alert) once, clearing
  // top chrome and the timeline card, then drop the request.
  useEffect(() => {
    if (!focusBounds || !patchedStyle) return;
    cameraRef.current?.fitBounds(focusBounds, {
      padding: { top: chrome.top + 28, right: chrome.right + 58, bottom: chrome.aboveTimeline + 20, left: chrome.left + 28 },
      duration: 600,
    });
    setFocusBounds(null);
  }, [focusBounds, patchedStyle, setFocusBounds, chrome]);

  if (!patchedStyle) return null;

  const handleLongPress = (event: NativeSyntheticEvent<PressEvent>) => {
    const [lon, lat] = event.nativeEvent.lngLat;
    onLongPress?.(lat, lon);
  };

  const handleRegionChange = (
    event: NativeSyntheticEvent<ViewStateChangeEvent>,
  ) => {
    const { center, zoom } = event.nativeEvent;
    zoomRef.current = zoom;
    onCameraChanged?.({
      lon: center[0],
      lat: center[1],
      zoom,
    });
  };

  return (
    <View style={styles.map}>
      <Map
        ref={mapRef}
        style={styles.map}
        mapStyle={patchedStyle}
        logo={false}
        attribution={true}
        attributionPosition={{ bottom: chrome.aboveTimeline + 8, left: chrome.left }}
        onLongPress={handleLongPress}
        onRegionIsChanging={trackCameraContinuously ? handleRegionChange : undefined}
        onRegionDidChange={handleRegionChange}
      >
        <Camera
          ref={cameraRef}
          initialViewState={{
            center: centerCoord,
            zoom: initialZoom,
          }}
        />
        {Children.toArray(children).filter(isValidElement)}
      </Map>

      {/* Manual zoom controls — pinch still works, this is for one-handed use. */}
      <MapChromeSurface
        style={[styles.zoomWrap, { right: chrome.right, bottom: chrome.aboveTimeline }]}
        fallbackStyle={styles.zoomFill}
        colorScheme="light"
        pointerEvents="box-none"
      >
        <Pressable
          onPress={() => zoomBy(+1)}
          style={({ pressed }) => [styles.zoomBtn, pressed && styles.zoomBtnPressed]}
          hitSlop={6}
          accessibilityRole="button"
          accessibilityLabel="Zoom in"
        >
          <SymbolView name={{ ios: "plus", android: "add" }} size={18} tintColor={ZOOM_ICON} weight="semibold" />
        </Pressable>
        <View style={styles.zoomDivider} />
        <Pressable
          onPress={() => zoomBy(-1)}
          style={({ pressed }) => [styles.zoomBtn, pressed && styles.zoomBtnPressed]}
          hitSlop={6}
          accessibilityRole="button"
          accessibilityLabel="Zoom out"
        >
          <SymbolView name={{ ios: "minus", android: "remove" }} size={18} tintColor={ZOOM_ICON} weight="semibold" />
        </Pressable>
      </MapChromeSurface>
    </View>
  );
}

const ZOOM_ICON = "#1a2030";

const styles = StyleSheet.create({
  map: {
    flex: 1,
  },
  zoomWrap: {
    position: "absolute",
    zIndex: 14,
    width: 48,
    borderRadius: 24,
    overflow: "hidden",
  },
  zoomFill: {
    backgroundColor: "rgba(255,255,255,0.92)",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(10,20,40,0.12)",
    shadowColor: "#000",
    shadowOpacity: 0.14,
    shadowRadius: 8,
    shadowOffset: { width: 0, height: 2 },
    elevation: 4,
  },
  zoomBtn: {
    width: 48,
    height: 48,
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  zoomBtnPressed: { opacity: 0.55 },
  zoomDivider: { height: StyleSheet.hairlineWidth, marginHorizontal: 10, backgroundColor: "rgba(10,20,40,0.18)" },
});
