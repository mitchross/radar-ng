import { GlassView, isLiquidGlassAvailable } from "expo-glass-effect";
import { useEffect, useState, type ReactNode } from "react";
import { AccessibilityInfo, View, type ColorValue, type StyleProp, type ViewStyle } from "react-native";

/**
 * True when map chrome should be Liquid Glass: iOS 26+ and Reduce
 * Transparency off. Elsewhere chrome keeps its translucent fill.
 */
export function useGlassChrome(): boolean {
  const available = isLiquidGlassAvailable();
  const [reduceTransparency, setReduceTransparency] = useState(false);
  useEffect(() => {
    if (!available) return;
    let mounted = true;
    AccessibilityInfo.isReduceTransparencyEnabled().then((on) => {
      if (mounted) setReduceTransparency(on);
    });
    const sub = AccessibilityInfo.addEventListener("reduceTransparencyChanged", setReduceTransparency);
    return () => {
      mounted = false;
      sub.remove();
    };
  }, [available]);
  return available && !reduceTransparency;
}

interface Props {
  /** Shape and layout, applied either way (size, radius, padding, position). */
  style?: StyleProp<ViewStyle>;
  /** Fill, border and shadow for the non-glass fallback only. */
  fallbackStyle?: StyleProp<ViewStyle>;
  /** Pin the glass appearance so the surface's text keeps its contrast. */
  colorScheme: "light" | "dark";
  tintColor?: ColorValue;
  interactive?: boolean;
  pointerEvents?: "auto" | "none" | "box-none" | "box-only";
  children?: ReactNode;
}

/** A panel or button over the map: Liquid Glass where available, a translucent fill otherwise. */
export function MapChromeSurface({
  style,
  fallbackStyle,
  colorScheme,
  tintColor,
  interactive,
  pointerEvents,
  children,
}: Props) {
  const glass = useGlassChrome();
  if (glass) {
    return (
      <GlassView
        style={style}
        glassEffectStyle="regular"
        colorScheme={colorScheme}
        tintColor={tintColor}
        isInteractive={interactive}
        pointerEvents={pointerEvents}
      >
        {children}
      </GlassView>
    );
  }
  return (
    <View style={[style, fallbackStyle]} pointerEvents={pointerEvents}>
      {children}
    </View>
  );
}
