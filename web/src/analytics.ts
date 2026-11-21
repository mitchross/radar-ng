const PREF_KEY = "rng.analytics";
const LIMIT = 64;

function umamiConfig(): { host: string; id: string } | undefined {
  const host = import.meta.env.VITE_UMAMI_HOST;
  const id = import.meta.env.VITE_UMAMI_WEBSITE_ID;
  if (!host || !id || !/^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(id)) return;
  try {
    const url = new URL(host);
    if (url.protocol === "https:" && url.origin === host) return { host, id };
  } catch { /* Invalid build config disables this provider. */ }
}
const UMAMI = umamiConfig();
export const ANALYTICS_AVAILABLE = Boolean(UMAMI);

function permitted(): boolean {
  try {
    const dnt = navigator.doNotTrack || (window as Window & { doNotTrack?: string }).doNotTrack;
    return localStorage.getItem(PREF_KEY) !== "0" && dnt !== "1" && dnt !== "yes";
  } catch { return false; }
}
let denied = !permitted();
let generation = 0;
const allowed = () => !denied && permitted();
export const analyticsEnabled = allowed;

type Payload = Record<string, unknown>;
type ProductEvent = { name?: string; data?: Payload };
type Provider = { ready?: (event: ProductEvent) => void; pending?: Promise<void>; queue: ProductEvent[]; pageview: boolean };
const um: Provider = { queue: [], pageview: false };
let umamiLoaded = false;
let replayWanted = false;
let recorderLoaded = false;

declare global {
  interface Window {
    umami?: { track: (payload: Payload) => Promise<void> };
    rngAnalyticsBeforeSend?: (type: string, payload: Payload) => Payload | null;
  }
}

// Map coordinates, search/hash state, titles and full referrers never enter event payloads.
const page = () => ({
  url: `${typeof location === "undefined" ? "" : location.origin}/`,
  referrer: "",
  title: "Radar NG",
});
const fields: Record<string, string[]> = {
  place_chosen: ["region"], layer_changed: ["layer"], map_style_changed: ["style"],
  timeline_range_changed: ["range"], playback_started: ["range", "layer"], playback_paused: ["range", "layer"],
  radar_product_changed: ["product"], palette_changed: ["palette"], overlay_toggled: ["overlay", "on"],
  overlay_inspected: ["layer"], point_inspected: ["layer", "source", "has_value"],
};
function safeData(name: string, data?: Payload): Payload | undefined {
  if (!data) return;
  return Object.fromEntries((fields[name] ?? []).flatMap(key => {
    const value = data[key];
    return typeof value === "boolean" || (typeof value === "string" && value.length <= 80)
      ? [[key, value]] : [];
  }));
}
function deliver(provider: Provider, event: ProductEvent) {
  if (!allowed()) return;
  if (provider.ready) {
    try { provider.ready(event); } catch { /* A tracker error must not break the page. */ }
  } else {
    if (provider.queue.length === LIMIT) provider.queue.shift();
    provider.queue.push(event);
  }
}
function flush(provider: Provider) {
  if (!allowed()) { provider.queue.length = 0; return; }
  if (!provider.pageview) {
    provider.pageview = true;
    deliver(provider, {});
  }
  for (const event of provider.queue.splice(0)) deliver(provider, event);
}
function loadUmami(epoch: number): Promise<void> {
  return new Promise((resolve, reject) => {
    window.rngAnalyticsBeforeSend = (type, payload) => allowed() && type === "event"
      ? { ...payload, ...page() } : null;
    const script = document.createElement("script");
    script.src = `${UMAMI!.host}/script.js`;
    Object.assign(script.dataset, {
      websiteId: UMAMI!.id, hostUrl: UMAMI!.host, autoTrack: "false", autoPageview: "false",
      doNotTrack: "true", errors: "false", performance: "false", excludeSearch: "true",
      excludeHash: "true", beforeSend: "rngAnalyticsBeforeSend",
    });
    const timeout = setTimeout(() => { script.remove(); reject(new Error("Tracker timeout")); }, 10000);
    script.onerror = () => { clearTimeout(timeout); script.remove(); reject(new Error("Tracker unavailable")); };
    script.onload = () => {
      clearTimeout(timeout);
      umamiLoaded = true;
      if (allowed() && epoch === generation && window.umami) {
        um.ready = event => {
          void window.umami!.track({ website: UMAMI!.id, ...page(), ...(event.name ? { name: event.name, data: event.data } : {}) }).catch(() => {});
        };
        flush(um);
        loadRecorder();
      }
      resolve();
    };
    document.head.append(script);
  });
}
// Session replay (rrweb). Umami's per-site settings decide masking and sampling; the recorder
// waits for the tracker's session, so the opt-outs above cover it too.
function loadRecorder() {
  if (!UMAMI || !umamiLoaded || !replayWanted || recorderLoaded || !allowed()) return;
  recorderLoaded = true;
  const script = document.createElement("script");
  script.src = `${UMAMI.host}/recorder.js`;
  Object.assign(script.dataset, { websiteId: UMAMI.id, hostUrl: UMAMI.host });
  document.head.append(script);
}
/** Start replay once the URL holds no location state (alertBounds is stripped first). */
export function startReplay(): void {
  replayWanted = true;
  loadRecorder();
}
function start(provider: Provider, load: (epoch: number) => Promise<void>) {
  if (provider.ready || provider.pending) return provider.pending;
  provider.pending = load(generation).catch(() => { provider.queue.length = 0; }).finally(() => { provider.pending = undefined; });
  return provider.pending;
}
export async function initAnalytics(): Promise<void> {
  if (!allowed()) return;
  if (UMAMI) await start(um, loadUmami);
}
function revoke() {
  denied = true;
  generation++;
  um.queue.length = 0;
  // A loaded tracker cannot be unloaded; reload so it never runs again on this page.
  if (umamiLoaded || um.pending) location.reload();
}
export async function setAnalyticsEnabled(on: boolean): Promise<void> {
  try { localStorage.setItem(PREF_KEY, on ? "1" : "0"); } catch { revoke(); return; }
  if (!on || !permitted()) { revoke(); return; }
  denied = false;
  await initAnalytics();
}
// Only in the browser; unit tests import this module under Node.
if (typeof window !== "undefined") {
  window.addEventListener("storage", event => {
    if (event.key !== PREF_KEY && event.key !== null) return;
    if (!permitted()) revoke();
    else void setAnalyticsEnabled(true);
    window.dispatchEvent(new Event("rng:analytics-changed"));
  });
}
export function track(event: string, props?: Payload): void {
  if (!allowed() || !Object.hasOwn(fields, event)) return;
  const payload = { name: event, data: safeData(event, props) };
  if (UMAMI) deliver(um, payload);
}
