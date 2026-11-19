/** Nice Day and Bonfire card for the forecast panel, from the forecast already loaded. */
import { escapeHtml } from "./html";
import { NICE_DAY_CREDIT, niceDayRows, niceDaySummary, type NiceDayHourly, type NiceDayWord } from "./shared/niceDay";

const WORD_CLASS: Record<NiceDayWord, string> = { Great: "great", Good: "good", Fair: "fair", Poor: "poor", "Do Not": "donot" };

/** Empty when no hour can be scored, so the panel never shows a hollow card. */
export function niceDayCardHtml(hourly: NiceDayHourly, start: number, todayKey: string): string {
  const rows = niceDayRows(niceDaySummary(hourly, start, todayKey));
  if (!rows.length) return "";
  return `<div class="section niceday">
      <div class="section-label">NICE DAY · BONFIRE</div>
      <div class="niceday-card">
        ${rows.map((r) => `<div class="niceday-row"><span>${escapeHtml(r.label)}<small>${escapeHtml(r.when)}</small></span><b class="nd-${WORD_CLASS[r.word]}">${escapeHtml(r.word)}</b></div>`).join("")}
        <p class="hint">${escapeHtml(NICE_DAY_CREDIT)}</p>
      </div>
    </div>`;
}
