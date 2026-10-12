/**
 * Product analytics and session replay through the self-hosted PostHog at
 * ingest-posthog.vanillax.me. Off unless VITE_POSTHOG_KEY is set at build time
 * (the project's public phc_ key), so dev and forks send nothing. Users can
 * also switch it off; the choice persists in localStorage.
 *
 * This is the site's only analytics: the Cloudflare Worker in talos
 * (infrastructure/networking/cloudflare-workers/posthog-inject.js) is not
 * deployed, and if it ever is, radar.vanillax.me belongs on its exclude list
 * so pages aren't recorded twice.
 */
import type { PostHog } from "posthog-js";

const KEY = import.meta.env.VITE_POSTHOG_KEY as string | undefined;
const HOST = (import.meta.env.VITE_POSTHOG_HOST as string | undefined) ?? "https://ingest-posthog.vanillax.me";
const PREF_KEY = "rng.analytics";

/** True when the build carries a key, so the settings toggle has something to switch. */
export const ANALYTICS_AVAILABLE = Boolean(KEY);

function readPref(): boolean {
  try {
    return localStorage.getItem(PREF_KEY) !== "0";
  } catch {
    return true;
  }
}

let enabled = readPref();

// Loaded in its own chunk after the map starts, so analytics never delays the radar.
let client: PostHog | null = null;
const queued: [string, Record<string, unknown> | undefined][] = [];

export async function initAnalytics(): Promise<void> {
  if (!KEY || !enabled || client) return;
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
  // PostHog persists its own opt-out flag; an earlier opt-out must not outlive a re-enable.
  if (posthog.has_opted_out_capturing()) posthog.opt_in_capturing();
  client = posthog;
  for (const [event, props] of queued.splice(0)) posthog.capture(event, props);
}

export function analyticsEnabled(): boolean {
  return enabled;
}

/** Persist the choice and apply it at once: opting out stops events and the session recording. */
export async function setAnalyticsEnabled(on: boolean): Promise<void> {
  enabled = on;
  try {
    localStorage.setItem(PREF_KEY, on ? "1" : "0");
  } catch {
    /* private mode */
  }
  if (!on) {
    queued.length = 0;
    client?.stopSessionRecording();
    client?.opt_out_capturing();
  } else if (client) {
    client.opt_in_capturing();
    client.startSessionRecording();
  } else await initAnalytics();
}

/** A named product event (layer switch, playback, search…). No-op when analytics is off. */
export function track(event: string, props?: Record<string, unknown>): void {
  if (!KEY || !enabled) return;
  if (client) client.capture(event, props);
  else queued.push([event, props]);
}
