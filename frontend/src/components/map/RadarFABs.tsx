/**
 * Cumulus radar right-rail controls: layer picker, pinpoint (inspector),
 * map-style picker. Glass-dark buttons; Apple-Weather-style popover with
 * icons + checkmark + layer-tinted background.
 */
import { useEffect, useRef, useState } from "react";
import { Animated, Easing, View, TouchableOpacity, StyleSheet, Text, Pressable } from "react-native";
import { useQueryClient } from "@tanstack/react-query";
import { useWeatherStore } from "../../stores/useWeatherStore";
import { pickNowFrameIndex } from "../../hooks/useManifest";
import { cumulus } from "../../lib/cumulusTheme";
import type { LayerType } from "../../types/weather";

type IconKind = "umbrella" | "thermo" | "dust" | "wind" | "bolt" | "layers" | "drop" | "cloud";

// Five layers, plain English, no acronyms. HRRR/Composite/CAPE/Precip-Type
// are still ingested in the backend (and feed the merged radar timeline) —
// they're just hidden from the picker because nobody asks "is the CAPE
// high today?" in normal life.
const LAYER_OPTIONS: {
  id: LayerType;
  name: string;
  icon: IconKind;
  tint: string;
  selfHostedOnly?: boolean;
}[] = [
  { id: "radar", name: "Radar", icon: "umbrella", tint: "#E8EFFA" },
  { id: "temperature", name: "Temperature", icon: "thermo", tint: "#D6F1FB", selfHostedOnly: true },
  { id: "wind", name: "Wind", icon: "wind", tint: "#DDE8F5", selfHostedOnly: true },
  { id: "precip-accum", name: "Rain Total", icon: "drop", tint: "#DEEAFA", selfHostedOnly: true },
  { id: "cloud", name: "Clouds", icon: "cloud", tint: "#ECECEF", selfHostedOnly: true },
];

export function RadarFABs({
  inspectorActive,
  onToggleInspector,
  onOpenStylePicker,
}: {
  inspectorActive: boolean;
  onToggleInspector: () => void;
  onOpenStylePicker?: () => void;
}) {
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const setActiveLayer = useWeatherStore((s) => s.setActiveLayer);
  const extrasVisible = useWeatherStore((s) => s.extrasVisible);
  const toggleExtras = useWeatherStore((s) => s.toggleExtras);
  const setCurrentFrameIndex = useWeatherStore((s) => s.setCurrentFrameIndex);
  const setIsPlaying = useWeatherStore((s) => s.setIsPlaying);
  const queryClient = useQueryClient();
  const [layerOpen, setLayerOpen] = useState(false);

  const [refreshing, setRefreshing] = useState(false);

  // Pause + immediately snap to "Now" using the current frame list, then
  // refetch the manifest. Use refetchQueries (not invalidateQueries) so the
  // request actually fires even when React Query still considers the data
  // fresh under refetchInterval — invalidate was a near-no-op for fast
  // taps. Also surface a spin state so the user gets visual confirmation.
  const onRefresh = async () => {
    if (refreshing) return;
    setRefreshing(true);
    setIsPlaying(false);
    const frames = useWeatherStore.getState().frames;
    setCurrentFrameIndex(pickNowFrameIndex(frames));
    try {
      await queryClient.refetchQueries({ queryKey: ["manifest"] });
    } finally {
      setRefreshing(false);
    }
  };

  const options = LAYER_OPTIONS;
  const activeOpt = options.find((o) => o.id === activeLayer) ?? options[0];
  const popoverBg = activeOpt?.tint ?? "#E8EFFA";

  return (
    <>
      <View style={styles.rail}>
        <GlassBtn active={layerOpen} onPress={() => setLayerOpen((v) => !v)}>
          <LayersIcon />
        </GlassBtn>
        <GlassBtn active={extrasVisible} onPress={toggleExtras}>
          <BoltIcon />
        </GlassBtn>
        <GlassBtn active={inspectorActive} onPress={onToggleInspector}>
          <CrosshairIcon />
        </GlassBtn>
        <GlassBtn onPress={onOpenStylePicker}>
          <MapStyleIcon />
        </GlassBtn>
        <GlassBtn onPress={onRefresh}>
          <RefreshIcon spinning={refreshing} />
        </GlassBtn>
      </View>

      {layerOpen && (
        <>
          <Pressable style={styles.scrim} onPress={() => setLayerOpen(false)} />
          <View style={[styles.panel, { backgroundColor: popoverBg }]}>
            {options.map((opt) => {
              const isActive = activeLayer === opt.id;
              return (
                <TouchableOpacity
                  key={opt.id}
                  onPress={() => {
                    setActiveLayer(opt.id);
                    setLayerOpen(false);
                  }}
                  style={styles.panelRow}
                  activeOpacity={0.6}
                >
                  <View style={styles.checkCol}>
                    {isActive && <CheckIcon />}
                  </View>
                  <View style={styles.iconCol}>
                    <LayerOptionIcon kind={opt.icon} />
                  </View>
                  <Text style={[styles.panelRowTitle, isActive && styles.panelRowTitleActive]}>
                    {opt.name}
                  </Text>
                </TouchableOpacity>
              );
            })}
          </View>
        </>
      )}
    </>
  );
}

function GlassBtn({
  children,
  onPress,
  active,
}: {
  children: React.ReactNode;
  onPress?: () => void;
  active?: boolean;
}) {
  return (
    <TouchableOpacity
      onPress={onPress}
      activeOpacity={0.8}
      style={[styles.btn, active && styles.btnActive]}
    >
      {children}
    </TouchableOpacity>
  );
}

/** ─── Icon primitives (View-based, no svg dep) ─────────────────────── */

function LayersIcon() {
  return (
    <View style={styles.iconBox}>
      <View style={[icons.diamond, { top: 0 }]} />
      <View style={[icons.diamond, { top: 5, opacity: 0.55 }]} />
    </View>
  );
}

function CrosshairIcon() {
  return (
    <View style={styles.iconBox}>
      <View style={icons.ring} />
      <View style={[icons.hLine, { top: 8.5 }]} />
      <View style={[icons.vLine, { left: 8.5 }]} />
      <View style={icons.pinDot} />
    </View>
  );
}

function BoltIcon() {
  // Lightning bolt — toggles the storm-cell + lightning-strike overlays.
  return (
    <View style={styles.iconBox}>
      <View style={icons.boltUpper} />
      <View style={icons.boltLower} />
    </View>
  );
}

function MapStyleIcon() {
  return (
    <View style={styles.iconBox}>
      <View style={icons.stackTop} />
      <View style={icons.stackBottom} />
    </View>
  );
}

function RefreshIcon({ spinning = false }: { spinning?: boolean }) {
  // Circular arrow — three-quarter ring + arrowhead at the top-right.
  // Spins while a refresh is in flight so the tap reads as "actually doing
  // something" — without this the FAB looked dead because the manifest
  // request usually completes before any visible state change.
  const rotation = useRef(new Animated.Value(0)).current;
  useEffect(() => {
    if (!spinning) {
      rotation.stopAnimation();
      rotation.setValue(0);
      return;
    }
    const loop = Animated.loop(
      Animated.timing(rotation, {
        toValue: 1,
        duration: 800,
        easing: Easing.linear,
        useNativeDriver: true,
      }),
    );
    loop.start();
    return () => loop.stop();
  }, [spinning, rotation]);
  const transform = [
    {
      rotate: rotation.interpolate({
        inputRange: [0, 1],
        outputRange: ["0deg", "360deg"],
      }),
    },
  ];
  return (
    <Animated.View style={[styles.iconBox, { transform }]}>
      <View style={icons.refreshRing} />
      <View style={icons.refreshNotch} />
      <View style={icons.refreshArrowA} />
      <View style={icons.refreshArrowB} />
    </Animated.View>
  );
}

function CheckIcon() {
  return (
    <View style={{ width: 14, height: 14 }}>
      <View
        style={{
          position: "absolute",
          left: 0,
          top: 6,
          width: 6,
          height: 2,
          backgroundColor: "#1a2030",
          borderRadius: 1,
          transform: [{ rotate: "45deg" }],
        }}
      />
      <View
        style={{
          position: "absolute",
          left: 3,
          top: 5,
          width: 10,
          height: 2,
          backgroundColor: "#1a2030",
          borderRadius: 1,
          transform: [{ rotate: "-45deg" }],
        }}
      />
    </View>
  );
}

function LayerOptionIcon({ kind }: { kind: IconKind }) {
  if (kind === "umbrella") {
    return (
      <View style={optIcon.box}>
        <View style={optIcon.umbrellaDome} />
        <View style={optIcon.umbrellaStem} />
        <View style={optIcon.umbrellaHook} />
      </View>
    );
  }
  if (kind === "thermo") {
    return (
      <View style={optIcon.box}>
        <View style={optIcon.thermoTube} />
        <View style={optIcon.thermoBulb} />
      </View>
    );
  }
  if (kind === "dust") {
    return (
      <View style={optIcon.box}>
        {Array.from({ length: 7 }).map((_, i) => (
          <View
            key={i}
            style={[
              optIcon.dustPt,
              {
                left: [3, 10, 16, 6, 13, 20, 9][i],
                top: [4, 7, 5, 12, 14, 11, 18][i],
              },
            ]}
          />
        ))}
      </View>
    );
  }
  if (kind === "wind") {
    return (
      <View style={optIcon.box}>
        <View style={[optIcon.windLine, { top: 4, width: 16 }]} />
        <View style={[optIcon.windLine, { top: 10, width: 20 }]} />
        <View style={[optIcon.windLine, { top: 16, width: 12 }]} />
      </View>
    );
  }
  if (kind === "bolt") {
    return (
      <View style={optIcon.box}>
        <View style={optIcon.boltTop} />
        <View style={optIcon.boltBottom} />
      </View>
    );
  }
  if (kind === "drop") {
    return (
      <View style={optIcon.box}>
        <View style={optIcon.dropBody} />
        <View style={optIcon.dropTip} />
      </View>
    );
  }
  if (kind === "cloud") {
    return (
      <View style={optIcon.box}>
        <View style={optIcon.cloudBase} />
        <View style={optIcon.cloudPuff1} />
        <View style={optIcon.cloudPuff2} />
      </View>
    );
  }
  return <View style={optIcon.box} />;
}

/** ─── Styles ───────────────────────────────────────────────────────── */

const styles = StyleSheet.create({
  rail: {
    position: "absolute",
    right: 12,
    top: 112,
    zIndex: 20,
    gap: 10,
  },
  btn: {
    width: 44,
    height: 44,
    borderRadius: 22,
    backgroundColor: "rgba(255,255,255,0.9)",
    alignItems: "center",
    justifyContent: "center",
    shadowColor: "#000",
    shadowOpacity: 0.12,
    shadowRadius: 6,
    shadowOffset: { width: 0, height: 2 },
    elevation: 3,
    borderWidth: 1,
    borderColor: "rgba(10,20,40,0.06)",
  },
  btnActive: {
    backgroundColor: cumulus.accent,
    borderColor: cumulus.accent,
  },
  iconBox: { width: 20, height: 20, alignItems: "center", justifyContent: "center" },

  scrim: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 0,
    bottom: 0,
    zIndex: 24,
  },
  panel: {
    position: "absolute",
    right: 68,
    top: 104,
    width: 230,
    borderRadius: 18,
    paddingVertical: 10,
    paddingHorizontal: 6,
    zIndex: 25,
    shadowColor: "#000",
    shadowOpacity: 0.18,
    shadowRadius: 14,
    shadowOffset: { width: 0, height: 6 },
    elevation: 8,
    borderWidth: 1,
    borderColor: "rgba(10,20,40,0.06)",
  },
  panelRow: {
    flexDirection: "row",
    alignItems: "center",
    paddingVertical: 10,
    paddingHorizontal: 6,
    gap: 10,
  },
  checkCol: { width: 16, alignItems: "center" },
  iconCol: { width: 22, alignItems: "center" },
  panelRowTitle: { color: "#1a2030", fontSize: 16, fontWeight: "400", flex: 1 },
  panelRowTitleActive: { fontWeight: "600" },
  panelHint: {
    color: "rgba(10,20,40,0.55)",
    fontSize: 11,
    paddingHorizontal: 12,
    paddingTop: 4,
    paddingBottom: 6,
  },
});

const icons = StyleSheet.create({
  diamond: {
    position: "absolute",
    width: 16,
    height: 16,
    borderWidth: 1.6,
    borderColor: "#1a2030",
    transform: [{ rotate: "45deg" }, { scaleY: 0.7 }],
  },
  ring: {
    position: "absolute",
    width: 14,
    height: 14,
    borderRadius: 7,
    borderWidth: 1.6,
    borderColor: "#1a2030",
  },
  hLine: {
    position: "absolute",
    width: 18,
    height: 1.6,
    backgroundColor: "#1a2030",
    borderRadius: 1,
    left: 1,
  },
  vLine: {
    position: "absolute",
    width: 1.6,
    height: 18,
    backgroundColor: "#1a2030",
    borderRadius: 1,
    top: 1,
  },
  pinDot: {
    position: "absolute",
    width: 3,
    height: 3,
    borderRadius: 1.5,
    backgroundColor: "#1a2030",
  },
  stackTop: {
    position: "absolute",
    top: 2,
    width: 16,
    height: 10,
    borderWidth: 1.6,
    borderColor: "#1a2030",
    borderRadius: 2,
    backgroundColor: "transparent",
  },
  stackBottom: {
    position: "absolute",
    top: 8,
    width: 16,
    height: 10,
    borderWidth: 1.6,
    borderColor: "#1a2030",
    borderRadius: 2,
    backgroundColor: "rgba(255,255,255,0.9)",
  },
  boltUpper: {
    position: "absolute",
    left: 6,
    top: 1,
    width: 6,
    height: 11,
    backgroundColor: "#1a2030",
    transform: [{ skewX: "-12deg" }],
  },
  boltLower: {
    position: "absolute",
    left: 9,
    top: 9,
    width: 6,
    height: 10,
    backgroundColor: "#1a2030",
    transform: [{ skewX: "-12deg" }],
  },
  refreshRing: {
    position: "absolute",
    left: 1,
    top: 1,
    width: 18,
    height: 18,
    borderRadius: 9,
    borderWidth: 1.8,
    borderColor: "#1a2030",
    backgroundColor: "transparent",
  },
  refreshNotch: {
    position: "absolute",
    right: 0,
    top: -1,
    width: 7,
    height: 7,
    backgroundColor: "rgba(255,255,255,0.9)",
  },
  refreshArrowA: {
    position: "absolute",
    right: 1,
    top: 1,
    width: 5,
    height: 1.8,
    backgroundColor: "#1a2030",
    transform: [{ rotate: "45deg" }],
    borderRadius: 1,
  },
  refreshArrowB: {
    position: "absolute",
    right: 1,
    top: 1,
    width: 1.8,
    height: 5,
    backgroundColor: "#1a2030",
    transform: [{ rotate: "45deg" }],
    borderRadius: 1,
  },
});

const optIcon = StyleSheet.create({
  box: { width: 22, height: 22 },
  umbrellaDome: {
    position: "absolute",
    left: 1,
    top: 4,
    width: 20,
    height: 10,
    borderTopLeftRadius: 10,
    borderTopRightRadius: 10,
    backgroundColor: "#1a2030",
  },
  umbrellaStem: {
    position: "absolute",
    left: 10.5,
    top: 12,
    width: 1.5,
    height: 7,
    backgroundColor: "#1a2030",
  },
  umbrellaHook: {
    position: "absolute",
    left: 7,
    top: 16,
    width: 5,
    height: 3,
    borderBottomLeftRadius: 3,
    borderLeftWidth: 1.5,
    borderBottomWidth: 1.5,
    borderColor: "#1a2030",
  },
  thermoTube: {
    position: "absolute",
    left: 10,
    top: 1,
    width: 3,
    height: 14,
    borderWidth: 1.4,
    borderColor: "#1a2030",
    borderRadius: 1.5,
    backgroundColor: "transparent",
  },
  thermoBulb: {
    position: "absolute",
    left: 8.5,
    top: 12,
    width: 6,
    height: 6,
    borderRadius: 3,
    backgroundColor: "#1a2030",
  },
  dustPt: {
    position: "absolute",
    width: 2.4,
    height: 2.4,
    borderRadius: 1.2,
    backgroundColor: "#1a2030",
  },
  windLine: {
    position: "absolute",
    left: 1,
    height: 1.6,
    backgroundColor: "#1a2030",
    borderRadius: 1,
  },
  boltTop: {
    position: "absolute",
    left: 6,
    top: 1,
    width: 7,
    height: 11,
    backgroundColor: "#1a2030",
    transform: [{ skewX: "-12deg" }],
  },
  boltBottom: {
    position: "absolute",
    left: 9,
    top: 9,
    width: 7,
    height: 11,
    backgroundColor: "#1a2030",
    transform: [{ skewX: "-12deg" }],
  },
  dropBody: {
    position: "absolute",
    left: 7,
    top: 8,
    width: 8,
    height: 10,
    borderRadius: 5,
    backgroundColor: "#1a2030",
  },
  dropTip: {
    position: "absolute",
    left: 9,
    top: 2,
    width: 4,
    height: 7,
    borderRadius: 2,
    backgroundColor: "#1a2030",
    transform: [{ rotate: "0deg" }, { scaleY: 1.3 }],
  },
  cloudBase: {
    position: "absolute",
    left: 2,
    top: 11,
    width: 18,
    height: 7,
    borderRadius: 4,
    backgroundColor: "#1a2030",
  },
  cloudPuff1: {
    position: "absolute",
    left: 5,
    top: 6,
    width: 8,
    height: 8,
    borderRadius: 4,
    backgroundColor: "#1a2030",
  },
  cloudPuff2: {
    position: "absolute",
    left: 10,
    top: 8,
    width: 7,
    height: 7,
    borderRadius: 3.5,
    backgroundColor: "#1a2030",
  },
});
