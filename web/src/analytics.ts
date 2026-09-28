/**
 * Product analytics and session replay through the self-hosted PostHog at
 * ingest-posthog.vanillax.me. Off unless VITE_POSTHOG_KEY is set at build time
 * (the project's public phc_ key), so dev and forks send nothing.
 *
 * This is the site's only analytics: the Cloudflare Worker in talos
 * (infrastructure/networking/cloudflare-workers/posthog-inject.js) is not
 * deployed, and if it ever is, radar.vanillax.me belongs on its exclude list
 * so pages aren't recorded twice.
 */
import type { PostHog } from "posthog-js";

const KEY = import.meta.env.VITE_POSTHOG_KEY as string | undefined;
const HOST = (import.meta.env.VITE_POSTHOG_HOST as string | undefined) ?? "https://ingest-posthog.vanillax.me";

// Loaded in its own chunk after the map starts, so analytics never delays the radar.
let client: PostHog | null = null;
const queued: [string, Record<string, unknown> | undefined][] = [];

export async function initAnalytics(): Promise<void> {
  if (!KEY) return;
  const { default: posthog } = await import("posthog-js");
  posthog.init(KEY, {
    api_host: HOST,
    ui_host: HOST,
    person_profiles: "identified_only",
    capture_pageview: true,
    capture_pageleave: true,
    // Replay: the map is a WebGL canvas, so recordings show the UI around it.
    session_recording: {
      maskAllInputs: true,
      recordCrossOriginIframes: false,
    },
    // Don't record the self-hosted API's JSON bodies into replays.
    capture_performance: { network_timing: true },
  });
  client = posthog;
  for (const [event, props] of queued.splice(0)) posthog.capture(event, props);
}

/** A named product event (layer switch, playback, search…). No-op when analytics is off. */
export function track(event: string, props?: Record<string, unknown>): void {
  if (!KEY) return;
  if (client) client.capture(event, props);
  else queued.push([event, props]);
}
