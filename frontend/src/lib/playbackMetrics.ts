/**
 * Playback smoothness metrics — measures the latency from a playback tick
 * (frame index advanced) to the next completed MapLibre frame render.
 *
 * This is a render-loop responsiveness proxy, not a tile-completion time:
 * MapLibre may finish a frame before the new raster tiles are decoded. It
 * pairs with `adb shell dumpsys gfxinfo` (UI-thread jank) for before/after
 * comparisons of the carousel work.
 *
 * One summary log event per play session ships via telemetry (Loki:
 * service radar-ng-mobile, msg "radar.playback.frameLatency").
 */
import { logEvent } from "./telemetry";

let pendingTickAt = 0;
let samples: number[] = [];

/** Called by the playback ticker right after advancing the frame index. */
export function markPlaybackTick(): void {
  pendingTickAt = performance.now();
}

/** Called from MapLibre's onDidFinishRenderingFrame. No-op unless a tick is pending. */
export function markFrameRendered(): void {
  if (pendingTickAt === 0) return;
  samples.push(performance.now() - pendingTickAt);
  pendingTickAt = 0;
}

function percentile(sorted: number[], p: number): number {
  const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[idx];
}

/** Called when playback stops; emits one summary event and resets. */
export function flushPlaybackMetrics(): void {
  pendingTickAt = 0;
  if (samples.length < 3) {
    samples = [];
    return;
  }
  const sorted = [...samples].sort((a, b) => a - b);
  logEvent("info", "radar.playback.frameLatency", {
    ticks: samples.length,
    p50_ms: Math.round(percentile(sorted, 50)),
    p95_ms: Math.round(percentile(sorted, 95)),
    max_ms: Math.round(sorted[sorted.length - 1]),
  });
  samples = [];
}
