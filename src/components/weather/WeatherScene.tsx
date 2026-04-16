/**
 * CARROT Premium-style weather scene illustration.
 * Renders a city skyline silhouette with weather-adaptive celestial objects
 * (sun, moon, stars, clouds, rain, snow, lightning) using only RN Views.
 */
import { View, StyleSheet, Dimensions } from "react-native";
import type { SceneType, WeatherTheme } from "../../lib/weatherTheme";

const SCREEN_WIDTH = Dimensions.get("window").width;
const SCENE_HEIGHT = 140;

// Building definitions: [xPercent, height, width]
const BUILDINGS: [number, number, number][] = [
  [0.02, 55, 28],
  [0.08, 75, 22],
  [0.14, 45, 30],
  [0.22, 90, 18],
  [0.27, 60, 26],
  [0.35, 70, 20],
  [0.40, 50, 32],
  [0.50, 100, 16],
  [0.54, 65, 24],
  [0.62, 80, 20],
  [0.68, 55, 28],
  [0.75, 95, 16],
  [0.79, 60, 22],
  [0.85, 45, 30],
  [0.92, 70, 20],
  [0.97, 50, 18],
];

// Window light positions within buildings (relative)
function BuildingWindows({ height, width, color }: { height: number; width: number; color: string }) {
  const rows = Math.floor((height - 12) / 10);
  const cols = Math.floor((width - 6) / 7);
  const windows: { top: number; left: number }[] = [];
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      // Randomly skip some windows (seeded by position)
      if ((r * 7 + c * 13) % 3 === 0) continue;
      windows.push({ top: 8 + r * 10, left: 4 + c * 7 });
    }
  }
  return (
    <>
      {windows.map((w, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            top: w.top,
            left: w.left,
            width: 3,
            height: 4,
            backgroundColor: color,
            opacity: ((i * 17) % 5) / 8 + 0.2,
          }}
        />
      ))}
    </>
  );
}

function Skyline({ theme }: { theme: WeatherTheme }) {
  return (
    <View style={skylineStyles.container}>
      {BUILDINGS.map(([xPct, h, w], i) => (
        <View
          key={i}
          style={[
            skylineStyles.building,
            {
              left: xPct * (SCREEN_WIDTH - 40),
              height: h,
              width: w,
              backgroundColor: theme.scene.skyline,
            },
          ]}
        >
          <BuildingWindows height={h} width={w} color={theme.scene.celestialGlow} />
        </View>
      ))}
      {/* Ground line */}
      <View style={[skylineStyles.ground, { backgroundColor: theme.scene.skyline }]} />
    </View>
  );
}

function Sun({ theme }: { theme: WeatherTheme }) {
  return (
    <View style={celestialStyles.sunContainer}>
      {/* Glow */}
      <View style={[celestialStyles.sunGlow, { backgroundColor: theme.scene.celestialGlow }]} />
      {/* Sun body */}
      <View style={[celestialStyles.sunBody, { backgroundColor: theme.scene.celestial }]} />
      {/* Rays */}
      {[0, 30, 60, 90, 120, 150].map((deg) => (
        <View
          key={deg}
          style={[
            celestialStyles.sunRay,
            {
              backgroundColor: theme.scene.celestial,
              transform: [{ rotate: `${deg}deg` }],
            },
          ]}
        />
      ))}
    </View>
  );
}

function Moon({ theme }: { theme: WeatherTheme }) {
  return (
    <View style={celestialStyles.moonContainer}>
      <View style={[celestialStyles.moonGlow, { backgroundColor: theme.scene.celestialGlow }]} />
      <View style={[celestialStyles.moonBody, { backgroundColor: theme.scene.celestial }]} />
      {/* Crescent shadow */}
      <View style={[celestialStyles.moonShadow, { backgroundColor: theme.gradient[1] }]} />
    </View>
  );
}

function Stars({ theme }: { theme: WeatherTheme }) {
  // Deterministic star positions
  const stars = [
    { x: 15, y: 8, s: 2.5 },
    { x: 45, y: 22, s: 1.5 },
    { x: 80, y: 12, s: 2 },
    { x: 120, y: 30, s: 1.5 },
    { x: 160, y: 10, s: 3 },
    { x: 200, y: 25, s: 1.5 },
    { x: 240, y: 15, s: 2 },
    { x: 280, y: 32, s: 1.5 },
    { x: 55, y: 38, s: 1 },
    { x: 140, y: 5, s: 2 },
    { x: 190, y: 40, s: 1 },
    { x: 310, y: 18, s: 2.5 },
    { x: 100, y: 42, s: 1 },
    { x: 260, y: 8, s: 1.5 },
    { x: 340, y: 28, s: 2 },
  ];
  return (
    <>
      {stars.map((star, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: star.x,
            top: star.y,
            width: star.s,
            height: star.s,
            borderRadius: star.s,
            backgroundColor: theme.scene.particles,
            opacity: 0.4 + (i % 4) * 0.15,
          }}
        />
      ))}
    </>
  );
}

function Clouds({ theme, count }: { theme: WeatherTheme; count: number }) {
  const clouds = [
    { x: 30, y: 20, w: 60, h: 18 },
    { x: 180, y: 10, w: 50, h: 14 },
    { x: 280, y: 28, w: 70, h: 20 },
    { x: 100, y: 35, w: 45, h: 12 },
  ].slice(0, count);

  return (
    <>
      {clouds.map((c, i) => (
        <View key={i} style={{ position: "absolute", left: c.x, top: c.y }}>
          {/* Cloud body — overlapping rounded rects */}
          <View
            style={{
              width: c.w,
              height: c.h,
              borderRadius: c.h / 2,
              backgroundColor: theme.scene.celestial,
              opacity: 0.25,
            }}
          />
          <View
            style={{
              position: "absolute",
              left: c.w * 0.2,
              top: -c.h * 0.4,
              width: c.w * 0.5,
              height: c.h * 1.0,
              borderRadius: c.h * 0.5,
              backgroundColor: theme.scene.celestial,
              opacity: 0.2,
            }}
          />
          <View
            style={{
              position: "absolute",
              left: c.w * 0.45,
              top: -c.h * 0.25,
              width: c.w * 0.35,
              height: c.h * 0.8,
              borderRadius: c.h * 0.4,
              backgroundColor: theme.scene.celestial,
              opacity: 0.18,
            }}
          />
        </View>
      ))}
    </>
  );
}

function RainDrops({ theme }: { theme: WeatherTheme }) {
  const drops = [
    { x: 25, y: 5 }, { x: 60, y: 15 }, { x: 95, y: 8 },
    { x: 130, y: 20 }, { x: 165, y: 3 }, { x: 200, y: 12 },
    { x: 235, y: 22 }, { x: 270, y: 6 }, { x: 305, y: 18 },
    { x: 45, y: 30 }, { x: 110, y: 35 }, { x: 180, y: 28 },
    { x: 250, y: 40 }, { x: 320, y: 32 }, { x: 80, y: 42 },
  ];
  return (
    <>
      {drops.map((d, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: d.x,
            top: d.y,
            width: 1.5,
            height: 8 + (i % 3) * 4,
            borderRadius: 1,
            backgroundColor: theme.scene.particles,
            opacity: 0.3 + (i % 4) * 0.1,
            transform: [{ rotate: "10deg" }],
          }}
        />
      ))}
    </>
  );
}

function SnowFlakes({ theme }: { theme: WeatherTheme }) {
  const flakes = [
    { x: 20, y: 10 }, { x: 55, y: 25 }, { x: 90, y: 5 },
    { x: 125, y: 30 }, { x: 160, y: 15 }, { x: 195, y: 35 },
    { x: 230, y: 8 }, { x: 265, y: 28 }, { x: 300, y: 18 },
    { x: 40, y: 40 }, { x: 145, y: 42 }, { x: 280, y: 38 },
  ];
  return (
    <>
      {flakes.map((f, i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: f.x,
            top: f.y,
            width: 3 + (i % 3),
            height: 3 + (i % 3),
            borderRadius: 3,
            backgroundColor: theme.scene.particles,
            opacity: 0.4 + (i % 3) * 0.15,
          }}
        />
      ))}
    </>
  );
}

function LightningBolt({ theme }: { theme: WeatherTheme }) {
  return (
    <View style={celestialStyles.lightningContainer}>
      {/* Bolt shape built from 3 angled segments */}
      <View style={[celestialStyles.boltSeg1, { backgroundColor: theme.scene.celestial }]} />
      <View style={[celestialStyles.boltSeg2, { backgroundColor: theme.scene.celestial }]} />
      <View style={[celestialStyles.boltSeg3, { backgroundColor: theme.scene.celestial }]} />
      {/* Glow behind bolt */}
      <View style={[celestialStyles.boltGlow, { backgroundColor: theme.scene.celestialGlow }]} />
    </View>
  );
}

export default function WeatherScene({ theme }: { theme: WeatherTheme }) {
  const sceneType = theme.sceneType;

  return (
    <View style={sceneStyles.container}>
      {/* Celestial objects layer */}
      <View style={sceneStyles.celestialLayer}>
        {(sceneType === "sunny") && <Sun theme={theme} />}
        {(sceneType === "night_clear" || sceneType === "night_cloudy") && (
          <>
            <Stars theme={theme} />
            <Moon theme={theme} />
          </>
        )}
        {(sceneType === "cloudy" || sceneType === "overcast" || sceneType === "foggy") && (
          <Clouds theme={theme} count={sceneType === "overcast" ? 4 : 3} />
        )}
        {sceneType === "night_cloudy" && (
          <>
            <Stars theme={theme} />
            <Clouds theme={theme} count={2} />
          </>
        )}
        {(sceneType === "rainy" || sceneType === "stormy") && (
          <>
            <Clouds theme={theme} count={4} />
            <RainDrops theme={theme} />
          </>
        )}
        {sceneType === "snowy" && (
          <>
            <Clouds theme={theme} count={3} />
            <SnowFlakes theme={theme} />
          </>
        )}
        {sceneType === "thunderstorm" && (
          <>
            <Clouds theme={theme} count={4} />
            <RainDrops theme={theme} />
            <LightningBolt theme={theme} />
          </>
        )}
      </View>

      {/* City skyline layer (foreground) */}
      <Skyline theme={theme} />
    </View>
  );
}

const sceneStyles = StyleSheet.create({
  container: {
    width: "100%",
    height: SCENE_HEIGHT,
    overflow: "hidden",
  },
  celestialLayer: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    bottom: 0,
  },
});

const skylineStyles = StyleSheet.create({
  container: {
    position: "absolute",
    bottom: 0,
    left: 20,
    right: 20,
    height: SCENE_HEIGHT,
  },
  building: {
    position: "absolute",
    bottom: 0,
    borderTopLeftRadius: 2,
    borderTopRightRadius: 2,
  },
  ground: {
    position: "absolute",
    bottom: 0,
    left: -20,
    right: -20,
    height: 1,
    opacity: 0.3,
  },
});

const celestialStyles = StyleSheet.create({
  // Sun
  sunContainer: {
    position: "absolute",
    top: 10,
    right: 60,
    width: 52,
    height: 52,
    alignItems: "center",
    justifyContent: "center",
  },
  sunGlow: {
    position: "absolute",
    width: 52,
    height: 52,
    borderRadius: 26,
    opacity: 0.4,
  },
  sunBody: {
    width: 28,
    height: 28,
    borderRadius: 14,
    opacity: 0.9,
  },
  sunRay: {
    position: "absolute",
    width: 2,
    height: 48,
    borderRadius: 1,
    opacity: 0.25,
  },
  // Moon
  moonContainer: {
    position: "absolute",
    top: 12,
    right: 55,
    width: 36,
    height: 36,
    alignItems: "center",
    justifyContent: "center",
  },
  moonGlow: {
    position: "absolute",
    width: 44,
    height: 44,
    borderRadius: 22,
    opacity: 0.3,
  },
  moonBody: {
    width: 28,
    height: 28,
    borderRadius: 14,
    opacity: 0.85,
  },
  moonShadow: {
    position: "absolute",
    width: 22,
    height: 22,
    borderRadius: 11,
    right: 2,
    top: 4,
    opacity: 0.9,
  },
  // Lightning
  lightningContainer: {
    position: "absolute",
    top: 30,
    left: SCREEN_WIDTH * 0.4,
    width: 20,
    height: 50,
  },
  boltSeg1: {
    position: "absolute",
    top: 0,
    left: 6,
    width: 4,
    height: 18,
    borderRadius: 1,
    transform: [{ rotate: "10deg" }],
    opacity: 0.8,
  },
  boltSeg2: {
    position: "absolute",
    top: 14,
    left: 2,
    width: 12,
    height: 3,
    borderRadius: 1,
    opacity: 0.8,
  },
  boltSeg3: {
    position: "absolute",
    top: 15,
    left: 3,
    width: 3,
    height: 22,
    borderRadius: 1,
    transform: [{ rotate: "-8deg" }],
    opacity: 0.7,
  },
  boltGlow: {
    position: "absolute",
    top: 5,
    left: 0,
    width: 20,
    height: 40,
    borderRadius: 10,
    opacity: 0.3,
  },
});
