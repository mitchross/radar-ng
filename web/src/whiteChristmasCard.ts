/** Seasonal White Christmas card for the forecast panel (Nov 15 to Dec 25). */
import { escapeHtml } from "./html";
import { WHITE_CHRISTMAS_CREDIT, whiteChristmasCard, type WhiteChristmasStation } from "./shared/whiteChristmas";

/** Empty outside the season or when no station is close enough, so the panel never shows a hollow card. */
export function whiteChristmasCardHtml(todayKey: string, station: WhiteChristmasStation | null, dotsOn: boolean): string {
  const card = whiteChristmasCard(todayKey, station);
  if (!card) return "";
  return `<div class="section-label">WHITE CHRISTMAS</div>
      <div class="wxmas-card">
        <b class="wxmas-pct">${escapeHtml(card.pctLabel)}</b>
        <div><span>${escapeHtml(card.headline)} ${escapeHtml(card.detail)}</span>
        <small>${escapeHtml(WHITE_CHRISTMAS_CREDIT)} · 1 inch or more on the ground</small>
        <button id="wxmas-map" class="wxmas-link" aria-pressed="${dotsOn}">${dotsOn ? "Hide stations on map" : "See stations on map"}</button></div>
      </div>`;
}
