/** "Snow at your spot" card for the forecast panel. Fixed height, so the skeleton and card never shift the panel. */
import { escapeHtml } from "./html";
import { snowSpotLabel, snowSpotView, type SnowClock, type SnowSpot } from "./shared/snowSpot";

const LABEL = `<div class="section-label">SNOW AT YOUR SPOT</div>`;

/** Same box as the card, shown while the answer loads. */
export function snowSpotSkeletonHtml(): string {
  return `${LABEL}<div class="snow-card snow-skeleton" aria-busy="true" aria-label="Loading snow at your spot"><i></i><i></i><i class="snow-skel-bars"></i><i></i></div>`;
}

export function snowSpotCardHtml(spot: SnowSpot, nowMs: number, clock: SnowClock): string {
  const v = snowSpotView(spot, nowMs, clock);
  const bars = v.bars
    .map((b) => `<span style="height:${Math.max(b.inches > 0 ? 6 : 2, Math.round(b.height * 100))}%"${b.inches > 0 ? ' class="on"' : ""}></span>`)
    .join("");
  const lines = v.lines.map((l) => `<span>${escapeHtml(l)}</span>`).join("");
  return `${LABEL}<div class="snow-card" role="group" aria-label="${escapeHtml(snowSpotLabel(v))}">
      <div class="snow-head">${escapeHtml(v.headline)}</div>
      <div class="snow-window">${v.window ? escapeHtml(v.window) : "&nbsp;"}</div>
      <div class="snow-bars" aria-hidden="true">${bars}</div>
      <div class="snow-lines">${lines}</div>
      <small>${v.credit ? escapeHtml(v.credit) : "&nbsp;"}</small>
    </div>`;
}
