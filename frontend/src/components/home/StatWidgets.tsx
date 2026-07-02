import React from "react";
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient as ExpoLinearGradient } from "expo-linear-gradient";
import {
  Canvas,
  Path,
  Circle,
  Line,
  LinearGradient as SkiaLinearGradient,
  vec,
  Skia,
  Group,
} from "@shopify/react-native-skia";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";

// 1. UV Index Bar
export function UVBar({ value }: { value: number }) {
  const pct = Math.min(1, value / 11);
  const color =
    value < 3
      ? "#4ADE80"
      : value < 6
      ? "#FFC14D"
      : value < 8
      ? "#FF9F2E"
      : value < 11
      ? "#FF4D6D"
      : "#B24BFF";
  return (
    <View style={styles.barContainer}>
      <ExpoLinearGradient
        colors={["#4ADE80", "#FFC14D", "#FF9F2E", "#FF4D6D", "#B24BFF"]}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 0 }}
        style={styles.gradientBar}
      />
      <View
        style={[
          styles.indicatorDot,
          {
            left: `${pct * 100}%`,
            backgroundColor: color,
            marginLeft: -4,
          },
        ]}
      />
    </View>
  );
}

// 2. Wind Dial
export function WindDial({ dir = 0 }: { dir: number }) {
  const { theme } = useWeatherClearTheme();
  const arrowPath = React.useMemo(() => {
    const path = Skia.Path.Make();
    path.moveTo(17, 5);
    path.lineTo(21, 16);
    path.lineTo(17, 13);
    path.lineTo(13, 16);
    path.close();
    return path;
  }, []);

  return (
    <Canvas style={styles.dialCanvas}>
      <Circle
        cx={17}
        cy={17}
        r={14}
        color={theme.colors.divider}
        style="stroke"
        strokeWidth={1}
      />
      <Group origin={{ x: 17, y: 17 }} transform={[{ rotate: dir }]}>
        <Path path={arrowPath} color={theme.colors.accent} />
      </Group>
    </Canvas>
  );
}

// 3. Humidity Fill Ring
export function FillRing({ value, color }: { value: number; color: string }) {
  const { theme } = useWeatherClearTheme();
  const R = 14;
  const arcPath = React.useMemo(() => {
    const path = Skia.Path.Make();
    // Bounding rect for circle: x=3, y=3, w=28, h=28
    path.addArc({ x: 3, y: 3, width: 28, height: 28 }, -90, value * 360);
    return path;
  }, [value]);

  return (
    <Canvas style={styles.dialCanvas}>
      <Circle
        cx={17}
        cy={17}
        r={R}
        color={theme.colors.divider}
        style="stroke"
        strokeWidth={3}
      />
      <Path
        path={arcPath}
        color={color}
        style="stroke"
        strokeWidth={3}
        strokeCap="round"
      />
    </Canvas>
  );
}

// 4. Visibility Bars
export function VisBars({ value }: { value: number }) {
  const { theme } = useWeatherClearTheme();
  const bars = 5;
  const filled = Math.round((value / 10) * bars);
  return (
    <View style={styles.visContainer}>
      {Array.from({ length: bars }).map((_, i) => (
        <View
          key={i}
          style={[
            styles.visBar,
            {
              height: (i + 1) * 3.6,
              backgroundColor:
                i < filled ? theme.colors.rain : theme.colors.divider,
            },
          ]}
        />
      ))}
    </View>
  );
}

// 5. Pressure Gauge
export function PressureGauge({ value }: { value: number }) {
  const { theme } = useWeatherClearTheme();
  // 980-1040 hPa mapping
  const pct = Math.max(0, Math.min(1, (value - 980) / 60));
  const angle = -180 + pct * 180;
  const rad = (angle * Math.PI) / 180;

  const trackPath = React.useMemo(() => {
    const path = Skia.Path.Make();
    path.addArc({ x: 4, y: 6, width: 32, height: 32 }, -180, 180);
    return path;
  }, []);

  const fillPath = React.useMemo(() => {
    const path = Skia.Path.Make();
    path.addArc({ x: 4, y: 6, width: 32, height: 32 }, -180, pct * 180);
    return path;
  }, [pct]);

  const needleX = 20 + 14 * Math.cos(rad);
  const needleY = 22 + 14 * Math.sin(rad);

  return (
    <Canvas style={styles.gaugeCanvas}>
      <Path
        path={trackPath}
        color={theme.colors.divider}
        style="stroke"
        strokeWidth={3}
        strokeCap="round"
      />
      <Path
        path={fillPath}
        color={theme.colors.accent}
        style="stroke"
        strokeWidth={3}
        strokeCap="round"
      />
      <Line
        p1={{ x: 20, y: 22 }}
        p2={{ x: needleX, y: needleY }}
        color={theme.colors.text}
        strokeWidth={1.5}
        strokeCap="round"
      />
      <Circle cx={20} cy={22} r={2} color={theme.colors.text} />
    </Canvas>
  );
}

// 6. AQI Bar
export function AQIBar({ value }: { value: number }) {
  const pct = Math.min(1, value / 300);
  const color =
    value < 50
      ? "#4ADE80"
      : value < 100
      ? "#FFC14D"
      : value < 150
      ? "#FF9F2E"
      : "#FF4D6D";
  return (
    <View style={styles.barContainer}>
      <ExpoLinearGradient
        colors={["#4ADE80", "#FFC14D", "#FF9F2E", "#FF4D6D"]}
        start={{ x: 0, y: 0 }}
        end={{ x: 1, y: 0 }}
        style={styles.gradientBar}
      />
      <View
        style={[
          styles.indicatorDot,
          {
            left: `${pct * 100}%`,
            backgroundColor: color,
            marginLeft: -4,
          },
        ]}
      />
    </View>
  );
}

// 7. Sun Arc Progress Tracker
export function SunArc({
  sunrise,
  sunset,
  progress,
}: {
  sunrise: string;
  sunset: string;
  progress: number;
}) {
  const { theme } = useWeatherClearTheme();
  const W = 280;
  const H = 90;

  // Math coordinates matching SVG formula
  const angle = Math.PI * progress;
  const cx = W / 2 - (W / 2) * Math.cos(angle);
  const cy = H - 10 - (H - 20) * Math.sin(angle);

  const arcPath = React.useMemo(() => {
    const path = Skia.Path.Make();
    // Bounding ellipse box: x=0, y=10, w=280, h=140
    path.addArc({ x: 0, y: 10, width: W, height: 140 }, -180, 180);
    return path;
  }, []);

  return (
    <View style={{ width: W, height: H + 20, alignSelf: "center", position: "relative" }}>
      <Canvas style={{ width: W, height: H + 10 }}>
        {/* Baseline */}
        <Line
          p1={{ x: 0, y: H - 10 }}
          p2={{ x: W, y: H - 10 }}
          color={theme.colors.divider}
          strokeWidth={1}
        />
        {/* Parabolic arc path */}
        <Path path={arcPath} style="stroke" strokeWidth={2}>
          <SkiaLinearGradient
            start={vec(0, 0)}
            end={vec(W, 0)}
            colors={[
              "rgba(240, 195, 78, 0.2)",
              "rgba(240, 195, 78, 0.9)",
              "rgba(223, 110, 58, 0.2)",
            ]}
          />
        </Path>
        {/* Sun dot and outer glow halo */}
        <Circle cx={cx} cy={cy} r={14} color="rgba(240, 195, 78, 0.25)" />
        <Circle cx={cx} cy={cy} r={8} color="#f0c34e" />
      </Canvas>

      <Text
        style={[
          styles.timeText,
          {
            left: 6,
            bottom: 2,
            color: theme.colors.textSecondary,
            fontFamily: theme.typography.mono,
          },
        ]}
      >
        {sunrise}
      </Text>
      <Text
        style={[
          styles.timeText,
          {
            right: 6,
            bottom: 2,
            textAlign: "right",
            color: theme.colors.textSecondary,
            fontFamily: theme.typography.mono,
          },
        ]}
      >
        {sunset}
      </Text>
      <Text
        style={[
          styles.sunHeader,
          {
            color: theme.colors.textMuted,
            fontFamily: theme.typography.mono,
          },
        ]}
      >
        DAYLIGHT
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  barContainer: {
    width: 52,
    height: 20,
    justifyContent: "flex-end",
    position: "relative",
  },
  gradientBar: {
    width: 52,
    height: 3,
    borderRadius: 1.5,
    opacity: 0.7,
    marginBottom: 4,
  },
  indicatorDot: {
    position: "absolute",
    bottom: 1.5,
    width: 8,
    height: 8,
    borderRadius: 4,
    borderWidth: 1.5,
    borderColor: "#fff",
    boxShadow: "0 1px 2px rgba(0,0,0,0.2)",
  },
  dialCanvas: {
    width: 34,
    height: 34,
  },
  visContainer: {
    flexDirection: "row",
    alignItems: "flex-end",
    width: 36,
    height: 20,
    gap: 2,
  },
  visBar: {
    width: 5,
    borderRadius: 1,
  },
  gaugeCanvas: {
    width: 40,
    height: 24,
  },
  timeText: {
    position: "absolute",
    fontSize: 10,
  },
  sunHeader: {
    position: "absolute",
    left: 0,
    right: 0,
    top: 14,
    textAlign: "center",
    fontSize: 10,
    fontWeight: "700",
    letterSpacing: 1.2,
  },
});
