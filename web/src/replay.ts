/** Replay data fetches and the picker/banner markup; every call goes to our own server. */
import { escapeHtml } from "./html";
import { parseReplayCatalogue, replayTileTemplate, type ReplayCase, type ReplayCatalogue } from "./shared/replayApi";
import { replayTimeLabel, replayClockLabel, type ReplayManifest } from "./shared/replaySource";
import type { ReplayWarningHistory } from "./shared/replayWarnings";

async function getJson(url: string, signal?: AbortSignal): Promise<unknown> {
  const res = await fetch(url, { signal });
  if (!res.ok) throw new Error(`${url}: ${res.status}`);
  return res.json();
}

export const fetchReplayCatalogue = async (signal?: AbortSignal): Promise<ReplayCatalogue> => parseReplayCatalogue(await getJson("/api/replays", signal));

export async function fetchReplayManifest(c: ReplayCase, signal?: AbortSignal): Promise<ReplayManifest> {
  const m = (await getJson(c.manifest_url, signal)) as ReplayManifest;
  if (!m || typeof m !== "object" || typeof m.layers !== "object" || !m.layers?.radar) throw new Error("replay manifest has no base radar frames");
  replayTileTemplate(m);
  return m;
}

export async function fetchReplayWarnings(c: ReplayCase, signal?: AbortSignal): Promise<ReplayWarningHistory> {
  const w = (await getJson(c.warnings_url, signal)) as ReplayWarningHistory;
  if (w?.type !== "FeatureCollection" || w.schema_version !== 1 || !Array.isArray(w.features) || w.case_id !== c.id) throw new Error("replay warnings do not match the contract");
  return w;
}

/** "Fri, Mar 24, 2023, 6:00 PM CDT to 10:00 PM CDT" in the case's zone. */
export const caseSpanLabel = (c: ReplayCase): string =>
  `${replayTimeLabel(Date.parse(c.start) / 1000, c.timezone)} to ${replayClockLabel(Date.parse(c.end) / 1000, c.timezone)}`;

export function pickerHtml(cases: readonly ReplayCase[]): string {
  if (cases.length === 0) return `<p class="hint">No replays are ready yet.</p>`;
  return cases
    .map((c) => `<button class="replay-case" data-case="${escapeHtml(c.id)}" data-revision="${escapeHtml(c.revision)}"><b>${escapeHtml(c.title)}</b><small>${escapeHtml(caseSpanLabel(c))}</small><span>${escapeHtml(c.summary)}</span></button>`)
    .join("");
}

export function bannerHtml(c: ReplayCase, error: string | null): string {
  return `<div><b>Historical replay</b><span>${escapeHtml(c.title)} · ${escapeHtml(caseSpanLabel(c))}</span>${error ? `<em role="alert">${escapeHtml(error)}</em>` : ""}</div>` +
    (error ? `<button id="replay-retry" class="retry" type="button">Retry</button>` : "") +
    `<button id="return-live" class="retry" type="button">Return to live</button>`;
}
