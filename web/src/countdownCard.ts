/** Holiday countdown card for the forecast panel. Loaded on demand: it pulls in astronomy-engine. */
import { escapeHtml } from "./html";
import { countdownRows, type CountdownPlace } from "./shared/holidays";

/** Empty when there is nothing to count down to, so the panel never shows a hollow card. */
export function countdownCardHtml(todayKey: string, place: CountdownPlace): string {
  const rows = countdownRows(todayKey, place, 3);
  if (!rows.length) return "";
  return `<div class="section-label">COMING UP</div>
      <div class="countdown-card">
        ${rows.map((r) => `<div class="countdown-row"><span>${escapeHtml(r.name)}<small>${escapeHtml(r.dateLabel)}${r.sunsetLabel ? ` · ${escapeHtml(r.sunsetLabel)}` : ""}</small></span><b>${escapeHtml(r.inLabel)}</b></div>`).join("")}
      </div>`;
}
