/** "Look up tonight" card for the forecast panel. Lazy-loaded with astronomy-engine. */
import { escapeHtml } from "./html";
import { inSkyWindow, skyTonightCard, tonight, type HourlyClouds, type IssReport, type SkyClock } from "./shared/skyTonight";

export interface SkyCardInput {
  nowMs: number;
  dateKey: string;
  lat: number;
  lon: number;
  report: IssReport | null;
  hourly: HourlyClouds | null;
  clock: SkyClock;
  /** The "Tonight" link was pressed: show the card outside the evening window too. */
  expanded: boolean;
}

/** Full card in the evening window or when expanded, else just the "Tonight" link. Empty in polar day or night. */
export function skyTonightCardHtml(input: SkyCardInput): string {
  const night = tonight(input.nowMs, input.dateKey, input.lat, input.lon);
  if (!night) return "";
  if (!input.expanded && !inSkyWindow(input.nowMs, night)) {
    return `<div class="sky-link-row"><span class="section-label">LOOK UP TONIGHT</span><button id="sky-tonight-open" class="sky-link">Tonight ›</button></div>`;
  }
  const card = skyTonightCard({ ...input, night });
  const verdict = card.verdictText
    ? `<div class="sky-verdict sky-${card.verdict}">${escapeHtml(card.verdictText)}</div>`
    : "";
  const list = (items: string[]) => items.map((t) => `<li>${escapeHtml(t)}</li>`).join("");
  const iss = card.passes.length ? `<ul class="sky-list">${list(card.passes)}</ul>` : "";
  const note = card.issNote ? `<p class="sky-note">${escapeHtml(card.issNote)}</p>` : "";
  const planets = card.planets.length
    ? `<ul class="sky-list">${list(card.planets)}</ul>`
    : `<p class="sky-note">No bright planets up after dark.</p>`;
  return `<div class="section-label">LOOK UP TONIGHT</div>
      <div class="sky-card">
        ${verdict}
        <b class="sky-head">ISS</b>${iss}${note}
        <b class="sky-head">Planets</b>${planets}
        <b class="sky-head">Moon</b><p class="sky-line">${escapeHtml(card.moon)}</p>
        <small>${escapeHtml(card.credit)} · passes 10° up or higher after dusk</small>
      </div>`;
}
