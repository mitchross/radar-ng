import { fetchWithTimeout } from "./api";
import { parseReplayCatalogue, replayTileTemplate, type ReplayCase, type ReplayCatalogue } from "./replayApi";
import { SourceGuard, sourceKey, type ReplayManifest, type SourceContext } from "./replaySource";
import type { ReplayWarningHistory } from "./replayWarnings";

/** Aborts in-flight replay requests on every source switch. */
export const replayGuard = new SourceGuard();

/** Cache keys: every replay key carries sourceKey and none shares the live ["manifest", url] key. */
export const replayCatalogueKey = (serverUrl: string) => ["replay-catalogue", serverUrl] as const;
export const replayManifestKey = (serverUrl: string, source: SourceContext) => ["replay-manifest", serverUrl, sourceKey(source)] as const;
export const replayWarningsKey = (serverUrl: string, source: SourceContext) => ["replay-warnings", serverUrl, sourceKey(source)] as const;

function anySignal(a: AbortSignal, b?: AbortSignal): AbortSignal {
  if (!b) return a;
  const ctrl = new AbortController();
  const abort = () => ctrl.abort();
  for (const s of [a, b]) {
    if (s.aborted) abort();
    else s.addEventListener("abort", abort);
  }
  return ctrl.signal;
}

async function getReplayJson(serverUrl: string, path: string, label: string, signal?: AbortSignal): Promise<unknown> {
  const ticket = replayGuard.ticket();
  const res = await fetchWithTimeout(`${serverUrl}${path}`, {}, anySignal(ticket.signal, signal), 20_000);
  if (!res.ok) throw new Error(`Replay ${label} error: ${res.status}`);
  const body = await res.json();
  if (!ticket.current()) throw new Error("Replay source changed");
  return body;
}

export async function fetchReplayCatalogue(serverUrl: string, signal?: AbortSignal): Promise<ReplayCatalogue> {
  return parseReplayCatalogue(await getReplayJson(serverUrl, "/api/replays", "catalogue", signal));
}

/** Throws on any contract drift: replay never falls back to the live manifest. */
export async function fetchReplayManifest(serverUrl: string, c: ReplayCase, signal?: AbortSignal): Promise<ReplayManifest> {
  const manifest = (await getReplayJson(serverUrl, c.manifest_url, "manifest", signal)) as ReplayManifest;
  if (!manifest || typeof manifest.layers !== "object" || manifest.layers === null) throw new Error("Replay manifest has no layers");
  replayTileTemplate(manifest);
  return manifest;
}

export async function fetchReplayWarnings(serverUrl: string, c: ReplayCase, signal?: AbortSignal): Promise<ReplayWarningHistory> {
  const body = (await getReplayJson(serverUrl, c.warnings_url, "warnings", signal)) as ReplayWarningHistory;
  if (body?.type !== "FeatureCollection" || body.schema_version !== 1 || !Array.isArray(body.features) || body.case_id !== c.id) {
    throw new Error("Replay warnings do not match the case");
  }
  return body;
}
