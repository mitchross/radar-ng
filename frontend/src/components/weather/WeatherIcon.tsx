/**
 * Cumulus WeatherIcon — View-only port of icons.jsx (no react-native-svg).
 * Icons are drawn with absolutely-positioned Views inside a square container
 * that scales linearly with the `size` prop.
 *
 * Supported kinds: sun, moon, partlyCloudy, cloudy, overcast, rain, heavyRain,
 * storm, snow, fog, hail.
 */
import { View } from "react-native";
import { cumulus, type IconKind } from "../../lib/cumulusTheme";

interface Props {
  kind: IconKind;
  size?: number;
  time?: "day" | "night";
}

/** Shared cloud silhouette — a stack of 3 rounded blobs. */
function Cloud({
  size,
  cx,
  cy,
  scale = 1,
  color = "#E8ECF5",
  shade = "#9AA4BE",
}: {
  size: number;
  cx: number;
  cy: number;
  scale?: number;
  color?: string;
  shade?: string;
}) {
  const u = size / 64;
  const w = 44 * scale * u;
  const h = 22 * scale * u;
  return (
    <View
      style={{
        position: "absolute",
        left: cx * u - w / 2,
        top: cy * u - h / 2,
        width: w,
        height: h,
      }}
    >
      {/* Main body */}
      <View
        style={{
          position: "absolute",
          left: 0,
          top: h * 0.35,
          width: w,
          height: h * 0.65,
          borderRadius: h * 0.32,
          backgroundColor: color,
        }}
      />
      {/* Left puff */}
      <View
        style={{
          position: "absolute",
          left: w * 0.1,
          top: h * 0.1,
          width: w * 0.45,
          height: h * 0.8,
          borderRadius: w * 0.24,
          backgroundColor: color,
        }}
      />
      {/* Right puff */}
      <View
        style={{
          position: "absolute",
          left: w * 0.42,
          top: 0,
          width: w * 0.5,
          height: h,
          borderRadius: w * 0.26,
          backgroundColor: color,
        }}
      />
      {/* Shadow on underside */}
      <View
        style={{
          position: "absolute",
          left: w * 0.05,
          bottom: 0,
          width: w * 0.9,
          height: h * 0.25,
          borderRadius: h * 0.12,
          backgroundColor: shade,
          opacity: 0.45,
        }}
      />
    </View>
  );
}

function Sun({ size, cx = 32, cy = 32, r = 11 }: { size: number; cx?: number; cy?: number; r?: number }) {
  const u = size / 64;
  const R = r * u;
  return (
    <>
      {/* Outer glow */}
      <View
        style={{
          position: "absolute",
          left: cx * u - (R + 6 * u),
          top: cy * u - (R + 6 * u),
          width: (R + 6 * u) * 2,
          height: (R + 6 * u) * 2,
          borderRadius: R + 6 * u,
          backgroundColor: cumulus.sun,
          opacity: 0.18,
        }}
      />
      {/* Inner glow */}
      <View
        style={{
          position: "absolute",
          left: cx * u - (R + 3 * u),
          top: cy * u - (R + 3 * u),
          width: (R + 3 * u) * 2,
          height: (R + 3 * u) * 2,
          borderRadius: R + 3 * u,
          backgroundColor: cumulus.sun,
          opacity: 0.3,
        }}
      />
      {/* Body */}
      <View
        style={{
          position: "absolute",
          left: cx * u - R,
          top: cy * u - R,
          width: R * 2,
          height: R * 2,
          borderRadius: R,
          backgroundColor: cumulus.sun,
        }}
      />
      {/* Highlight */}
      <View
        style={{
          position: "absolute",
          left: (cx - 3) * u - R * 0.35,
          top: (cy - 3) * u - R * 0.35,
          width: R * 0.7,
          height: R * 0.7,
          borderRadius: R * 0.35,
          backgroundColor: "#FFE7A8",
          opacity: 0.9,
        }}
      />
    </>
  );
}

function Moon({ size, cx = 32, cy = 30, r = 11 }: { size: number; cx?: number; cy?: number; r?: number }) {
  const u = size / 64;
  const R = r * u;
  return (
    <>
      <View
        style={{
          position: "absolute",
          left: cx * u - (R + 4 * u),
          top: cy * u - (R + 4 * u),
          width: (R + 4 * u) * 2,
          height: (R + 4 * u) * 2,
          borderRadius: R + 4 * u,
          backgroundColor: "#C7B8FF",
          opacity: 0.15,
        }}
      />
      <View
        style={{
          position: "absolute",
          left: cx * u - R,
          top: cy * u - R,
          width: R * 2,
          height: R * 2,
          borderRadius: R,
          backgroundColor: "#E8DFFF",
        }}
      />
      {/* Crescent shadow */}
      <View
        style={{
          position: "absolute",
          left: (cx + 4) * u - R * 0.95,
          top: (cy - 2) * u - R * 0.95,
          width: R * 1.9,
          height: R * 1.9,
          borderRadius: R * 0.95,
          backgroundColor: cumulus.background,
        }}
      />
    </>
  );
}

function RainDrops({ size, y, intensity = "light" }: { size: number; y: number; intensity?: "light" | "heavy" }) {
  const u = size / 64;
  const drops =
    intensity === "heavy"
      ? [{ x: 16 }, { x: 22 }, { x: 30 }, { x: 38 }, { x: 44 }]
      : [{ x: 20 }, { x: 30 }, { x: 40 }];
  return (
    <>
      {drops.map((d, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: d.x * u,
            top: (y + (i % 3)) * u,
            width: 2.4 * u,
            height: 6 * u,
            borderRadius: 1.2 * u,
            backgroundColor: cumulus.rain,
            opacity: 0.8,
          }}
        />
      ))}
    </>
  );
}

function SnowFlakes({ size, y }: { size: number; y: number }) {
  const u = size / 64;
  return (
    <>
      {[18, 28, 38, 46].map((x, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: x * u - 2 * u,
            top: (y + (i % 2) * 3) * u - 2 * u,
            width: 4 * u,
            height: 4 * u,
            borderRadius: 2 * u,
            backgroundColor: "#E8F4FF",
            opacity: 0.6 + (i % 3) * 0.15,
          }}
        />
      ))}
    </>
  );
}

function Lightning({ size, x = 30, y = 40 }: { size: number; x?: number; y?: number }) {
  const u = size / 64;
  return (
    <>
      <View
        style={{
          position: "absolute",
          left: (x - 2) * u,
          top: y * u,
          width: 3 * u,
          height: 10 * u,
          backgroundColor: "#FFD93D",
          transform: [{ rotate: "10deg" }],
        }}
      />
      <View
        style={{
          position: "absolute",
          left: (x - 3) * u,
          top: (y + 8) * u,
          width: 6 * u,
          height: 3 * u,
          backgroundColor: "#FFD93D",
        }}
      />
      <View
        style={{
          position: "absolute",
          left: (x - 1) * u,
          top: (y + 10) * u,
          width: 3 * u,
          height: 10 * u,
          backgroundColor: "#FFD93D",
          transform: [{ rotate: "-8deg" }],
        }}
      />
    </>
  );
}

function FogBars({ size }: { size: number }) {
  const u = size / 64;
  // Min 3.5px bar height so the icon stays visible at small sizes (28px and below).
  const h = Math.max(3.5, 5 * u);
  const bars = [
    { x: 10, y: 16, w: 44, o: 0.95 },
    { x: 14, y: 26, w: 40, o: 0.75 },
    { x: 8,  y: 36, w: 46, o: 0.9 },
    { x: 16, y: 46, w: 36, o: 0.65 },
  ];
  return (
    <>
      {bars.map((b, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: b.x * u,
            top: b.y * u,
            width: b.w * u,
            height: h,
            borderRadius: h / 2,
            backgroundColor: "#E8ECF5",
            opacity: b.o,
          }}
        />
      ))}
    </>
  );
}

function Hailstones({ size, y }: { size: number; y: number }) {
  const u = size / 64;
  return (
    <>
      {[22, 32, 42].map((x, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: x * u - 2.8 * u,
            top: (y + (i === 1 ? 4 : 0)) * u - 2.8 * u,
            width: 5.6 * u,
            height: 5.6 * u,
            borderRadius: 2.8 * u,
            backgroundColor: "#E8F4FF",
          }}
        />
      ))}
    </>
  );
}

export default function WeatherIcon({ kind, size = 40, time = "day" }: Props) {
  return (
    <View style={{ width: size, height: size }}>
      {kind === "sun" && <Sun size={size} />}
      {kind === "moon" && <Moon size={size} />}

      {kind === "partlyCloudy" && (
        <>
          {time === "night" ? <Moon size={size} cx={22} cy={22} r={8} /> : <Sun size={size} cx={22} cy={22} r={8} />}
          <Cloud size={size} cx={38} cy={40} scale={0.85} />
        </>
      )}

      {kind === "cloudy" && <Cloud size={size} cx={32} cy={32} scale={1.1} />}

      {kind === "overcast" && (
        <>
          <Cloud size={size} cx={26} cy={26} scale={0.9} color="#AEB6CC" shade="#6F7895" />
          <Cloud size={size} cx={38} cy={38} scale={1.0} />
        </>
      )}

      {kind === "rain" && (
        <>
          <Cloud size={size} cx={32} cy={24} scale={1.05} />
          <RainDrops size={size} y={38} intensity="light" />
        </>
      )}

      {kind === "heavyRain" && (
        <>
          <Cloud size={size} cx={32} cy={22} scale={1.1} color="#9AA4BE" shade="#5C6682" />
          <RainDrops size={size} y={36} intensity="heavy" />
        </>
      )}

      {kind === "storm" && (
        <>
          <Cloud size={size} cx={32} cy={22} scale={1.1} color="#7C87A8" shade="#4A5372" />
          <Lightning size={size} x={32} y={36} />
          <RainDrops size={size} y={46} intensity="light" />
        </>
      )}

      {kind === "snow" && (
        <>
          <Cloud size={size} cx={32} cy={24} scale={1.05} />
          <SnowFlakes size={size} y={40} />
        </>
      )}

      {kind === "fog" && <FogBars size={size} />}

      {kind === "hail" && (
        <>
          <Cloud size={size} cx={32} cy={22} scale={1.05} color="#9AA4BE" shade="#5C6682" />
          <Hailstones size={size} y={44} />
        </>
      )}
    </View>
  );
}
