/**
 * Advanced-mode "Conditions" tiles. Every tile has the same anatomy (symbol +
 * label, value, caption, a full-width visual) so the grid reads as one set.
 * Tiles with no source data are left out rather than shown as "Unavailable".
 */
import React, { useMemo } from "react";
import { View, Text, StyleSheet } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { SymbolView, type SymbolViewProps } from "expo-symbols";
import { Canvas, Path, Circle, LinearGradient as SkiaLinearGradient, vec, Skia, Group } from "@shopify/react-native-skia";
import { useWeatherClearTheme } from "../../theme/WeatherClearThemeProvider";
import type { WeatherClearTheme } from "../../theme/weatherClearTheme";
import { getUVInfo, getWindDirection } from "../../lib/cumulusTheme";
import { formatDegrees } from "../../lib/temperature";
import {
  daylightNote,
  dewPointFeel,
  feelsLikeNote,
  hPaToInHg,
  pressureLabel,
  windToward,
} from "../../lib/conditions";

type SymbolName = SymbolViewProps["name"];

export interface ConditionsInput {
  uv: number | null;
  windMph: number | null;
  gustMph: number | null;
  windFromDeg: number | null;
  humidity: number | null;
  /** Dew point for display (user's unit) and in °F for the comfort wording. */
  dew: number | null;
  dewF: number | null;
  pressureHPa: number | null;
  rainTodayIn: number | null;
  rainTomorrowIn: number | null;
  /** Next 12 hours of precipitation amounts (inches). */
  rainNext: (number | null)[];
  feels: number | null;
  /** Air temperature in the user's unit. */
  actual: number | null;
  feelsF: number | null;
  actualF: number | null;
  sunrise: Date | null;
  sunset: Date | null;
  now: Date;
}

const UV_COLORS = ["#4ADE80", "#FFC14D", "#FF9F2E", "#FF4D6D", "#B24BFF"] as const;
const PRESSURE_MIN_INHG = 28.9;
const PRESSURE_MAX_INHG = 30.7;

export function ConditionTiles(props: ConditionsInput) {
  const { theme } = useWeatherClearTheme();
  const styles = useMemo(() => createStyles(theme), [theme]);
  const { uv, windMph, gustMph, windFromDeg, humidity, dew, dewF, pressureHPa } = props;
  const uvInfo = uv === null ? null : getUVInfo(uv);
  const inHg = pressureHPa === null ? null : hPaToInHg(pressureHPa);
  const rainPeak = Math.max(0.05, ...props.rainNext.map((v) => v ?? 0));
  const sunTimes = (d: Date | null) =>
    d ? d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }).toLowerCase() : "—";
  const dayProgress =
    props.sunrise && props.sunset
      ? Math.max(0, Math.min(1, (props.now.getTime() - props.sunrise.getTime()) / (props.sunset.getTime() - props.sunrise.getTime())))
      : null;

  return (
    <>
      <View style={styles.grid}>
        {uv !== null && uvInfo ? (
          <Tile styles={styles} symbol={{ ios: "sun.max.fill", android: "wb_sunny" }} label="UV INDEX"
            value={String(Math.round(uv))} caption={uvInfo.label} captionColor={uvInfo.color}
            a11y={`UV index ${Math.round(uv)}, ${uvInfo.label}`}>
            <ScaleBar styles={styles} pct={Math.min(1, uv / 11)} colors={UV_COLORS} markerColor={uvInfo.color} />
          </Tile>
        ) : null}

        {windMph !== null ? (
          <Tile styles={styles} symbol={{ ios: "wind", android: "air" }} label="WIND"
            value={String(windMph)} unit="mph"
            caption={[
              windFromDeg == null ? null : `From ${getWindDirection(windFromDeg)}`,
              gustMph == null ? null : `gusts ${gustMph}`,
            ].filter(Boolean).join(" · ") || "—"}
            a11y={`Wind ${windMph} miles per hour${windFromDeg == null ? "" : ` from the ${getWindDirection(windFromDeg)}`}${gustMph == null ? "" : `, gusts ${gustMph}`}`}
            aside={windFromDeg == null ? null : <Compass theme={theme} towardDeg={windToward(windFromDeg)} />}
          />
        ) : null}

        {humidity !== null ? (
          <Tile styles={styles} symbol={{ ios: "humidity.fill", android: "water_drop" }} label="HUMIDITY"
            value={String(humidity)} unit="%"
            caption={dew === null ? "—" : `Dew point ${formatDegrees(dew)}${dewF == null ? "" : ` · ${dewPointFeel(dewF)}`}`}
            a11y={`Humidity ${humidity} percent${dew === null ? "" : `, dew point ${dew} degrees`}`}>
            <ScaleBar styles={styles} pct={humidity / 100} colors={[theme.colors.surfaceMuted, theme.colors.rain]} markerColor={theme.colors.rain} />
          </Tile>
        ) : null}

        {inHg !== null && pressureHPa !== null ? (
          <Tile styles={styles} symbol={{ ios: "gauge.with.dots.needle.33percent", android: "speed" }} label="PRESSURE"
            value={inHg.toFixed(2)} unit="inHg" caption={pressureLabel(pressureHPa)}
            a11y={`Pressure ${inHg.toFixed(2)} inches of mercury, ${pressureLabel(pressureHPa)}`}>
            <ScaleBar styles={styles}
              pct={Math.max(0, Math.min(1, (inHg - PRESSURE_MIN_INHG) / (PRESSURE_MAX_INHG - PRESSURE_MIN_INHG)))}
              colors={[theme.colors.cold, theme.colors.surfaceMuted, theme.colors.hot]} markerColor={theme.colors.text} />
          </Tile>
        ) : null}

        {props.rainTodayIn !== null ? (
          <Tile styles={styles} symbol={{ ios: "cloud.rain.fill", android: "rainy" }} label="RAIN TODAY"
            value={props.rainTodayIn.toFixed(2)} unit="in"
            caption={props.rainTomorrowIn === null ? "Next 12 hours below" : `Tomorrow ${props.rainTomorrowIn.toFixed(2)}″`}
            a11y={`Rain today ${props.rainTodayIn.toFixed(2)} inches${props.rainTomorrowIn === null ? "" : `, tomorrow ${props.rainTomorrowIn.toFixed(2)} inches`}`}>
            <View style={styles.rainBars}>
              {props.rainNext.map((v, i) => (
                <View key={i} style={styles.rainSlot}>
                  <View style={[styles.rainBar, { height: v ? Math.max(2, (v / rainPeak) * 22) : 2, opacity: v ? 1 : 0.25 }]} />
                </View>
              ))}
            </View>
          </Tile>
        ) : null}

        {props.feels !== null ? (
          <Tile styles={styles} symbol={{ ios: "thermometer.medium", android: "thermostat" }} label="FEELS LIKE"
            value={formatDegrees(props.feels)}
            caption={props.actualF === null || props.feelsF === null ? "—" : feelsLikeNote(props.actualF, props.feelsF)}
            a11y={`Feels like ${props.feels} degrees${props.actual === null ? "" : `, actual ${props.actual} degrees`}`}>
            {props.actual !== null ? (
              <Text style={styles.footnote}>Actual {formatDegrees(props.actual)}</Text>
            ) : null}
          </Tile>
        ) : null}
      </View>

      {dayProgress !== null ? (
        <View style={styles.sunCard} accessible
          accessibilityLabel={`Sunrise ${sunTimes(props.sunrise)}, sunset ${sunTimes(props.sunset)}. ${daylightNote(props.now, props.sunrise, props.sunset) ?? ""}`}>
          <View style={styles.tileHeader}>
            <SymbolView name={{ ios: "sun.horizon.fill", android: "wb_twilight" }} size={13} tintColor={theme.colors.textMuted} type="hierarchical" />
            <Text style={styles.tileLabel}>DAYLIGHT</Text>
            <Text style={[styles.tileLabel, styles.sunNote]}>{daylightNote(props.now, props.sunrise, props.sunset)}</Text>
          </View>
          <SunArc theme={theme} progress={dayProgress} />
          <View style={styles.sunTimes}>
            <SunTime styles={styles} theme={theme} symbol={{ ios: "sunrise.fill", android: "wb_twilight" }} label="Sunrise" time={sunTimes(props.sunrise)} />
            <SunTime styles={styles} theme={theme} symbol={{ ios: "sunset.fill", android: "wb_twilight" }} label="Sunset" time={sunTimes(props.sunset)} alignEnd />
          </View>
        </View>
      ) : null}
    </>
  );
}

type Styles = ReturnType<typeof createStyles>;

function Tile({ styles, symbol, label, value, unit, caption, captionColor, a11y, aside, children }: {
  styles: Styles;
  symbol: SymbolName;
  label: string;
  value: string;
  unit?: string;
  caption: string;
  captionColor?: string;
  a11y: string;
  aside?: React.ReactNode;
  children?: React.ReactNode;
}) {
  const { theme } = useWeatherClearTheme();
  return (
    <View style={styles.tile} accessible accessibilityLabel={a11y}>
      <View style={styles.tileHeader}>
        <SymbolView name={symbol} size={13} tintColor={theme.colors.textMuted} type="hierarchical" />
        <Text style={styles.tileLabel}>{label}</Text>
      </View>
      <View style={styles.tileBody}>
        <View style={{ flex: 1 }}>
          <View style={styles.valueRow}>
            <Text style={styles.value}>{value}</Text>
            {unit ? <Text style={styles.unit}>{unit}</Text> : null}
          </View>
          <Text style={[styles.caption, captionColor ? { color: captionColor } : null]} numberOfLines={2}>{caption}</Text>
        </View>
        {aside}
      </View>
      {children ? <View style={styles.visual}>{children}</View> : null}
    </View>
  );
}

function ScaleBar({ styles, pct, colors, markerColor }: {
  styles: Styles;
  pct: number;
  colors: readonly [string, string, ...string[]];
  markerColor: string;
}) {
  return (
    <View style={styles.scale}>
      <LinearGradient colors={colors} start={{ x: 0, y: 0 }} end={{ x: 1, y: 0 }} style={styles.scaleTrack} />
      <View style={[styles.scaleMarker, { left: `${pct * 100}%`, backgroundColor: markerColor }]} />
    </View>
  );
}

/** Arrow points the way the wind blows; "N" marks north. */
function Compass({ theme, towardDeg }: { theme: WeatherClearTheme; towardDeg: number }) {
  const S = 46;
  const c = S / 2;
  const arrow = useMemo(() => {
    const p = Skia.Path.Make();
    p.moveTo(c, 7);
    p.lineTo(c + 6, c + 7);
    p.lineTo(c, c + 3);
    p.lineTo(c - 6, c + 7);
    p.close();
    return p;
  }, [c]);
  return (
    <View style={{ width: S, height: S }}>
      <Canvas style={{ width: S, height: S }}>
        <Circle cx={c} cy={c} r={c - 2} color={theme.colors.divider} style="stroke" strokeWidth={1.5} />
        {/* Skia rotates in radians. */}
        <Group origin={{ x: c, y: c }} transform={[{ rotate: (towardDeg * Math.PI) / 180 }]}>
          <Path path={arrow} color={theme.colors.accent} />
        </Group>
      </Canvas>
      <Text style={{ position: "absolute", top: -1, left: 0, right: 0, textAlign: "center", fontSize: 8, fontWeight: "800", color: theme.colors.textMuted }}>N</Text>
    </View>
  );
}

function SunArc({ theme, progress }: { theme: WeatherClearTheme; progress: number }) {
  const W = 280;
  const H = 76;
  const angle = Math.PI * progress;
  const cx = W / 2 - (W / 2 - 8) * Math.cos(angle);
  const cy = H - 6 - (H - 16) * Math.sin(angle);
  const arc = useMemo(() => {
    const p = Skia.Path.Make();
    p.addArc({ x: 8, y: 10, width: W - 16, height: (H - 16) * 2 }, -180, 180);
    return p;
  }, []);
  return (
    <Canvas style={{ width: W, height: H, alignSelf: "center", marginTop: 6 }}>
      <Path path={arc} style="stroke" strokeWidth={2}>
        <SkiaLinearGradient start={vec(0, 0)} end={vec(W, 0)}
          colors={["rgba(240,195,78,0.25)", "rgba(240,195,78,0.95)", "rgba(223,110,58,0.25)"]} />
      </Path>
      <Circle cx={cx} cy={cy} r={13} color="rgba(240,195,78,0.25)" />
      <Circle cx={cx} cy={cy} r={7} color="#f0c34e" />
    </Canvas>
  );
}

function SunTime({ styles, theme, symbol, label, time, alignEnd }: {
  styles: Styles; theme: WeatherClearTheme; symbol: SymbolName; label: string; time: string; alignEnd?: boolean;
}) {
  return (
    <View style={[styles.sunTime, alignEnd ? { flexDirection: "row-reverse" } : null]}>
      <SymbolView name={symbol} size={22} tintColor="#e3a33a" type="hierarchical" />
      <View style={alignEnd ? { alignItems: "flex-end" } : null}>
        <Text style={styles.tileLabel}>{label.toUpperCase()}</Text>
        <Text style={styles.sunValue}>{time}</Text>
      </View>
    </View>
  );
}

function createStyles(theme: WeatherClearTheme) {
  return StyleSheet.create({
    grid: { flexDirection: "row", flexWrap: "wrap", marginHorizontal: 16, gap: 10 },
    tile: {
      flexBasis: "47%",
      flexGrow: 1,
      minHeight: 128,
      backgroundColor: theme.colors.surface,
      borderRadius: 18,
      borderWidth: 1,
      borderColor: theme.colors.border,
      padding: 14,
    },
    tileHeader: { flexDirection: "row", alignItems: "center", gap: 6 },
    tileLabel: {
      color: theme.colors.textMuted,
      fontSize: 10,
      fontWeight: "700",
      letterSpacing: 0.7,
      fontFamily: theme.typography.ui,
    },
    tileBody: { flexDirection: "row", alignItems: "flex-start", marginTop: 8, gap: 8 },
    valueRow: { flexDirection: "row", alignItems: "baseline", gap: 3 },
    value: { color: theme.colors.text, fontSize: 30, fontFamily: theme.typography.display, fontWeight: "500" },
    unit: { color: theme.colors.textMuted, fontSize: 13, fontWeight: "600", fontFamily: theme.typography.ui },
    caption: { color: theme.colors.textSecondary, fontSize: 12, fontWeight: "500", fontFamily: theme.typography.ui, marginTop: 2 },
    visual: { marginTop: "auto", paddingTop: 12 },
    scale: { height: 12, justifyContent: "center" },
    scaleTrack: { height: 5, borderRadius: 3 },
    scaleMarker: {
      position: "absolute",
      width: 12,
      height: 12,
      marginLeft: -6,
      borderRadius: 6,
      borderWidth: 2,
      borderColor: theme.colors.surface,
      boxShadow: "0 1px 3px rgba(0,0,0,0.25)",
    },
    rainBars: { flexDirection: "row", alignItems: "flex-end", height: 24, gap: 2 },
    rainSlot: { flex: 1, justifyContent: "flex-end" },
    rainBar: { borderRadius: 2, backgroundColor: theme.colors.rain },
    footnote: { color: theme.colors.textMuted, fontSize: 12, fontWeight: "600", fontFamily: theme.typography.ui },
    sunCard: {
      marginHorizontal: 16,
      marginTop: 10,
      backgroundColor: theme.colors.surface,
      borderRadius: 18,
      borderWidth: 1,
      borderColor: theme.colors.border,
      padding: 14,
    },
    sunNote: { marginLeft: "auto", letterSpacing: 0, fontWeight: "600", color: theme.colors.textSecondary },
    sunTimes: { flexDirection: "row", justifyContent: "space-between", marginTop: 4 },
    sunTime: { flexDirection: "row", alignItems: "center", gap: 8 },
    sunValue: { color: theme.colors.text, fontSize: 20, fontFamily: theme.typography.display, fontWeight: "500", marginTop: 1 },
  });
}
