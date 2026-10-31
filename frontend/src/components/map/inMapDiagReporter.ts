/**
 * Sends the in-map engine's stats as one `inmap_diag` telemetry event: at most once a minute while
 * In-map is active, and once for every error stage the first time it is seen. Attributes are counts,
 * zoom and errors only (see inMapDiagAttributes), so telemetryPrivacy has nothing to strip.
 */
import {
  diagEventDue,
  inMapDiagAttributes,
  inMapStats,
  setInMapErrorListener,
  type InMapStage,
} from "../../lib/inMapDiag";
import { logEvent } from "../../lib/telemetry";

/** How often the timer checks whether a periodic event is due; the once-a-minute limit is diagEventDue's. */
const CHECK_MS = 10_000;

export function startInMapDiagReporter(): () => void {
  let lastSentAt: number | null = null;
  const seenStages = new Set<InMapStage>();
  const send = (reason: string, newStage: InMapStage | null) => {
    const now = Date.now();
    if (!diagEventDue(lastSentAt, now, newStage, seenStages)) return;
    lastSentAt = now;
    logEvent(inMapStats.lastError ? "warn" : "info", "inmap_diag", inMapDiagAttributes(inMapStats, reason));
  };
  setInMapErrorListener((stage) => send(`error:${stage}`, stage));
  const id = setInterval(() => send("periodic", null), CHECK_MS);
  return () => {
    clearInterval(id);
    setInMapErrorListener(undefined);
  };
}
