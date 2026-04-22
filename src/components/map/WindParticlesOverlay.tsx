/**
 * Wind particles — Skia-canvas overlay of advecting particles that follow
 * HRRR's U/V vector field at 10m AGL. Runs on the UI thread via Reanimated
 * worklets; no JS-thread work per frame.
 *
 *   - particle count: PARTICLE_COUNT
 *   - lifetime: particles fade in, advect, fade out, respawn
 *   - trail: short line from previous to current position
 *
 * Rendering: all particles are baked into ONE compound Skia Path per frame —
 * 1200 React components per frame would wreck the reconciler.
 *
 * Camera tracking: parent owns a SharedCamera (lon/lat/zoom shared values)
 * written from MapLibre's `onCameraChanged`. Each frame re-projects particles
 * via a local web-mercator implementation (no cross-thread map API calls).
 */
import {
  Canvas,
  Path,
  Skia,
} from "@shopify/react-native-skia";
import { useEffect, useMemo } from "react";
import { StyleSheet, useWindowDimensions } from "react-native";
import {
  useSharedValue,
  useDerivedValue,
  useFrameCallback,
  runOnUI,
  type SharedValue,
} from "react-native-reanimated";
import {
  sampleWindField,
  type WindField,
  useWindField,
} from "../../hooks/useWindField";
import { useWeatherStore } from "../../stores/useWeatherStore";

const PARTICLE_COUNT = 1200;
const LIFETIME_FRAMES = 90; // ~1.5s at 60fps
const SPEED_SCALE = 0.0008; // deg lon/lat per frame per mph; tune for feel
// One frame's drift in degrees is sub-pixel at typical zoom levels, so the
// rendered "head" is a thicker stub at the current position plus a short
// tail drawn back along the wind vector. TRAIL_FRAMES is the multiplier on
// per-frame drift used to compute the tail length.
const TRAIL_FRAMES = 18;
// Seed particles in a box centered on the camera. Without this, 1200
// particles spread across CONUS leaves only a handful in a ~1-2° viewport.
const SEED_HALF_DEG = 3.0;

export interface SharedCamera {
  lon: SharedValue<number>;
  lat: SharedValue<number>;
  zoom: SharedValue<number>;
}

export function useSharedCamera(
  initLon: number,
  initLat: number,
  initZoom = 7,
): SharedCamera {
  const lon = useSharedValue(initLon);
  const lat = useSharedValue(initLat);
  const zoom = useSharedValue(initZoom);
  return { lon, lat, zoom };
}

interface ParticleBuffer {
  lats: Float32Array;
  lons: Float32Array;
  prevLats: Float32Array;
  prevLons: Float32Array;
  ages: Int32Array;
  speeds: Float32Array;
}

export function WindParticlesOverlay({
  enabled,
  camera,
}: {
  enabled: boolean;
  camera: SharedCamera;
}) {
  const { width, height } = useWindowDimensions();
  const frames = useWeatherStore((s) => s.frames);
  const currentFrameIndex = useWeatherStore((s) => s.currentFrameIndex);
  const activeLayer = useWeatherStore((s) => s.activeLayer);
  const dataSource = useWeatherStore((s) => s.dataSource);

  const frame = frames[currentFrameIndex];
  const timestamp = frame?.path ?? null;
  const shouldFetch =
    enabled && dataSource === "selfhosted" &&
    (activeLayer === "wind" || activeLayer === "radar" || activeLayer === "radar-hrrr");
  const { data: fieldData } = useWindField(shouldFetch ? timestamp : null);
  const field: WindField | null =
    fieldData && fieldData.ok ? (fieldData as WindField) : null;

  // Typed-array particle buffer — allocated once, mutated in place by the
  // UI-thread frame callback.
  const particles = useMemo<ParticleBuffer>(
    () => ({
      lats: new Float32Array(PARTICLE_COUNT),
      lons: new Float32Array(PARTICLE_COUNT),
      prevLats: new Float32Array(PARTICLE_COUNT),
      prevLons: new Float32Array(PARTICLE_COUNT),
      ages: new Int32Array(PARTICLE_COUNT),
      speeds: new Float32Array(PARTICLE_COUNT),
    }),
    [],
  );
  // Bump tick every frame so useDerivedValue recomputes.
  const tick = useSharedValue(0);

  // (Re)seed particles when the wind field changes. Seeds around the current
  // camera center so most particles land in-viewport.
  useEffect(() => {
    if (!field) return;
    runOnUI(() => {
      "worklet";
      const camLon = camera.lon.value;
      const camLat = camera.lat.value;
      const lonMin = Math.max(field.lon_min, camLon - SEED_HALF_DEG);
      const lonMax = Math.min(field.lon_max, camLon + SEED_HALF_DEG);
      const latMin = Math.max(field.lat_min, camLat - SEED_HALF_DEG);
      const latMax = Math.min(field.lat_max, camLat + SEED_HALF_DEG);
      for (let i = 0; i < PARTICLE_COUNT; i++) {
        particles.lats[i] = latMin + Math.random() * (latMax - latMin);
        particles.lons[i] = lonMin + Math.random() * (lonMax - lonMin);
        particles.prevLats[i] = particles.lats[i];
        particles.prevLons[i] = particles.lons[i];
        particles.ages[i] = Math.floor(Math.random() * LIFETIME_FRAMES);
        particles.speeds[i] = 0;
      }
    })();
  }, [field, particles, camera]);

  // Advect every particle each frame.
  useFrameCallback(() => {
    "worklet";
    if (!enabled || !field) return;
    const camLon = camera.lon.value;
    const camLat = camera.lat.value;
    const lonMin = Math.max(field.lon_min, camLon - SEED_HALF_DEG);
    const lonMax = Math.min(field.lon_max, camLon + SEED_HALF_DEG);
    const latMin = Math.max(field.lat_min, camLat - SEED_HALF_DEG);
    const latMax = Math.min(field.lat_max, camLat + SEED_HALF_DEG);
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const age = particles.ages[i] + 1;
      // Respawn either on age or on drift out of the seed box — keeps particle
      // density following the camera as the user pans.
      const outOfBox =
        particles.lons[i] < lonMin ||
        particles.lons[i] > lonMax ||
        particles.lats[i] < latMin ||
        particles.lats[i] > latMax;
      if (age >= LIFETIME_FRAMES || outOfBox) {
        particles.lats[i] = latMin + Math.random() * (latMax - latMin);
        particles.lons[i] = lonMin + Math.random() * (lonMax - lonMin);
        particles.prevLats[i] = particles.lats[i];
        particles.prevLons[i] = particles.lons[i];
        particles.ages[i] = 0;
        continue;
      }
      const [u, v] = sampleWindField(
        field,
        particles.lats[i],
        particles.lons[i],
      );
      particles.prevLats[i] = particles.lats[i];
      particles.prevLons[i] = particles.lons[i];
      particles.lons[i] += u * SPEED_SCALE;
      particles.lats[i] += v * SPEED_SCALE;
      particles.speeds[i] = Math.sqrt(u * u + v * v);
      particles.ages[i] = age;
    }
    tick.value += 1;
  }, true);

  // Inline the loop into each derived value. Calling buildPath across the
  // worklet boundary was dropping calls (the outer () => buildPath(...) arrow
  // is auto-wrapped as a worklet but buildPath itself wasn't being treated
  // as one, leading to empty paths).
  const pathSlow = useDerivedValue(() => {
    "worklet";
    void tick.value;
    const p = Skia.Path.Make();
    if (!field) return p;
    const scale = (256 * Math.pow(2, camera.zoom.value)) / (2 * Math.PI);
    const cx = width / 2;
    const cy = height / 2;
    const centerProj = projectLngLat(camera.lon.value, camera.lat.value, scale);
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const s = particles.speeds[i];
      if (s >= 15) continue;
      const dLon = particles.lons[i] - particles.prevLons[i];
      const dLat = particles.lats[i] - particles.prevLats[i];
      const tailLon = particles.lons[i] - dLon * TRAIL_FRAMES;
      const tailLat = particles.lats[i] - dLat * TRAIL_FRAMES;
      const p1 = projectLngLat(tailLon, tailLat, scale);
      const p2 = projectLngLat(particles.lons[i], particles.lats[i], scale);
      const x1 = cx + (p1.x - centerProj.x);
      const y1 = cy + (p1.y - centerProj.y);
      const x2 = cx + (p2.x - centerProj.x);
      const y2 = cy + (p2.y - centerProj.y);
      if (x2 < -20 || x2 > width + 20 || y2 < -20 || y2 > height + 20) continue;
      p.moveTo(x1, y1);
      p.lineTo(x2, y2);
    }
    return p;
  });

  const pathMed = useDerivedValue(() => {
    "worklet";
    void tick.value;
    const p = Skia.Path.Make();
    if (!field) return p;
    const scale = (256 * Math.pow(2, camera.zoom.value)) / (2 * Math.PI);
    const cx = width / 2;
    const cy = height / 2;
    const centerProj = projectLngLat(camera.lon.value, camera.lat.value, scale);
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const s = particles.speeds[i];
      if (s < 15 || s >= 30) continue;
      const dLon = particles.lons[i] - particles.prevLons[i];
      const dLat = particles.lats[i] - particles.prevLats[i];
      const tailLon = particles.lons[i] - dLon * TRAIL_FRAMES;
      const tailLat = particles.lats[i] - dLat * TRAIL_FRAMES;
      const p1 = projectLngLat(tailLon, tailLat, scale);
      const p2 = projectLngLat(particles.lons[i], particles.lats[i], scale);
      const x1 = cx + (p1.x - centerProj.x);
      const y1 = cy + (p1.y - centerProj.y);
      const x2 = cx + (p2.x - centerProj.x);
      const y2 = cy + (p2.y - centerProj.y);
      if (x2 < -20 || x2 > width + 20 || y2 < -20 || y2 > height + 20) continue;
      p.moveTo(x1, y1);
      p.lineTo(x2, y2);
    }
    return p;
  });

  const pathFast = useDerivedValue(() => {
    "worklet";
    void tick.value;
    const p = Skia.Path.Make();
    if (!field) return p;
    const scale = (256 * Math.pow(2, camera.zoom.value)) / (2 * Math.PI);
    const cx = width / 2;
    const cy = height / 2;
    const centerProj = projectLngLat(camera.lon.value, camera.lat.value, scale);
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const s = particles.speeds[i];
      if (s < 30 || s >= 50) continue;
      const dLon = particles.lons[i] - particles.prevLons[i];
      const dLat = particles.lats[i] - particles.prevLats[i];
      const tailLon = particles.lons[i] - dLon * TRAIL_FRAMES;
      const tailLat = particles.lats[i] - dLat * TRAIL_FRAMES;
      const p1 = projectLngLat(tailLon, tailLat, scale);
      const p2 = projectLngLat(particles.lons[i], particles.lats[i], scale);
      const x1 = cx + (p1.x - centerProj.x);
      const y1 = cy + (p1.y - centerProj.y);
      const x2 = cx + (p2.x - centerProj.x);
      const y2 = cy + (p2.y - centerProj.y);
      if (x2 < -20 || x2 > width + 20 || y2 < -20 || y2 > height + 20) continue;
      p.moveTo(x1, y1);
      p.lineTo(x2, y2);
    }
    return p;
  });

  const pathExtreme = useDerivedValue(() => {
    "worklet";
    void tick.value;
    const p = Skia.Path.Make();
    if (!field) return p;
    const scale = (256 * Math.pow(2, camera.zoom.value)) / (2 * Math.PI);
    const cx = width / 2;
    const cy = height / 2;
    const centerProj = projectLngLat(camera.lon.value, camera.lat.value, scale);
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      const s = particles.speeds[i];
      if (s < 50) continue;
      const dLon = particles.lons[i] - particles.prevLons[i];
      const dLat = particles.lats[i] - particles.prevLats[i];
      const tailLon = particles.lons[i] - dLon * TRAIL_FRAMES;
      const tailLat = particles.lats[i] - dLat * TRAIL_FRAMES;
      const p1 = projectLngLat(tailLon, tailLat, scale);
      const p2 = projectLngLat(particles.lons[i], particles.lats[i], scale);
      const x1 = cx + (p1.x - centerProj.x);
      const y1 = cy + (p1.y - centerProj.y);
      const x2 = cx + (p2.x - centerProj.x);
      const y2 = cy + (p2.y - centerProj.y);
      if (x2 < -20 || x2 > width + 20 || y2 < -20 || y2 > height + 20) continue;
      p.moveTo(x1, y1);
      p.lineTo(x2, y2);
    }
    return p;
  });

  if (!enabled || !field) return null;

  return (
    <Canvas
      style={[StyleSheet.absoluteFill, styles.canvas]}
      pointerEvents="none"
    >
      {/* dark outline pass — ensures trails are visible on ANY background
          (green heatmap, blue water, dark basemap) */}
      <Path path={pathSlow} color="rgba(10,15,30,0.85)" style="stroke" strokeWidth={4.0} />
      <Path path={pathMed} color="rgba(10,15,30,0.85)" style="stroke" strokeWidth={4.4} />
      <Path path={pathFast} color="rgba(10,15,30,0.85)" style="stroke" strokeWidth={4.8} />
      <Path path={pathExtreme} color="rgba(10,15,30,0.85)" style="stroke" strokeWidth={5.2} />
      {/* bright core pass */}
      <Path path={pathSlow} color="rgba(240,248,255,1)" style="stroke" strokeWidth={2.0} />
      <Path path={pathMed} color="rgba(120,255,220,1)" style="stroke" strokeWidth={2.4} />
      <Path path={pathFast} color="rgba(255,220,90,1)" style="stroke" strokeWidth={2.8} />
      <Path path={pathExtreme} color="rgba(255,70,110,1)" style="stroke" strokeWidth={3.2} />
    </Canvas>
  );
}

function projectLngLat(lon: number, lat: number, scale: number) {
  "worklet";
  const x = ((lon * Math.PI) / 180) * scale;
  const latRad = Math.max(-1.4844, Math.min(1.4844, (lat * Math.PI) / 180));
  const y = -Math.log(Math.tan(Math.PI / 4 + latRad / 2)) * scale;
  return { x, y };
}

const styles = StyleSheet.create({
  canvas: { zIndex: 5 },
});
