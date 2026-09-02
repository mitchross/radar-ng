const LIVE_MAX_AGE_SECONDS = 10 * 60;

export type RadarStatusTone = "live" | "stale" | "offline" | "unavailable";

export interface RadarStatus {
  label: string;
  accessibilityLabel: string;
  tone: RadarStatusTone;
}

interface RadarStatusInput {
  frameTimeSeconds: number | null;
  refreshFailed: boolean;
  loading: boolean;
  nowMilliseconds?: number;
}

function ageLabel(ageMinutes: number): { badge: string; spoken: string } {
  if (ageMinutes < 60) {
    return {
      badge: `${ageMinutes}M`,
      spoken: `${ageMinutes} minute${ageMinutes === 1 ? "" : "s"}`,
    };
  }

  const ageHours = Math.floor(ageMinutes / 60);
  if (ageHours < 24) {
    return {
      badge: `${ageHours}H`,
      spoken: `${ageHours} hour${ageHours === 1 ? "" : "s"}`,
    };
  }

  const ageDays = Math.floor(ageHours / 24);
  return {
    badge: `${ageDays}D`,
    spoken: `${ageDays} day${ageDays === 1 ? "" : "s"}`,
  };
}

/** Derive a truthful status from the displayed observation, not manifest age. */
export function radarStatus({
  frameTimeSeconds,
  refreshFailed,
  loading,
  nowMilliseconds = Date.now(),
}: RadarStatusInput): RadarStatus {
  if (frameTimeSeconds === null) {
    if (refreshFailed) {
      return {
        label: "UNAVAILABLE",
        accessibilityLabel: "Radar unavailable",
        tone: "unavailable",
      };
    }
    return loading
      ? {
          label: "LOADING",
          accessibilityLabel: "Radar loading",
          tone: "unavailable",
        }
      : {
          label: "UNAVAILABLE",
          accessibilityLabel: "Radar unavailable",
          tone: "unavailable",
        };
  }

  if (refreshFailed) {
    return {
      label: "OFFLINE",
      accessibilityLabel: "Radar offline. Showing the last saved observation",
      tone: "offline",
    };
  }

  const nowSeconds = Math.floor(nowMilliseconds / 1000);
  const ageSeconds = Math.max(0, nowSeconds - frameTimeSeconds);
  if (ageSeconds <= LIVE_MAX_AGE_SECONDS) {
    return {
      label: "LIVE",
      accessibilityLabel: "Live radar available",
      tone: "live",
    };
  }

  const age = ageLabel(Math.max(1, Math.floor(ageSeconds / 60)));
  return {
    label: `UPDATED ${age.badge} AGO`,
    accessibilityLabel: `Radar updated ${age.spoken} ago`,
    tone: "stale",
  };
}
