import type { NowcastSkillHeadline } from "./nowcastTypes";

/** Fewer runs than this is a single storm's first minutes, not a track record. */
export const MIN_RUNS_FOR_SKILL = 6;

export interface NowcastSkillView {
  /** Share of observed rain the forecast caught, 0–100. */
  hitRatePct: number;
  /** Share of forecast rain that never arrived, 0–100, or null when nothing was forecast. */
  falseAlarmPct: number | null;
  leadMinutes: number;
  windowHours: number;
  runs: number;
  /** True when the motion forecast beat "assume the radar stays put". */
  beatsPersistence: boolean | null;
  /** One line for the Nowcast screen. */
  sentence: string;
  /** A few words for a banner. */
  short: string;
}

/**
 * Turn the server's verified-accuracy headline into words. Null when there is
 * nothing honest to say yet: no headline, too few runs, or no rain observed.
 */
export function describeNowcastSkill(
  skill: NowcastSkillHeadline | null | undefined,
): NowcastSkillView | null {
  if (!skill || skill.pod == null || !Number.isFinite(skill.pod)) return null;
  if (!Number.isFinite(skill.runs) || skill.runs < MIN_RUNS_FOR_SKILL) return null;
  const hitRatePct = Math.round(Math.max(0, Math.min(1, skill.pod)) * 100);
  const falseAlarmPct =
    skill.far == null || !Number.isFinite(skill.far)
      ? null
      : Math.round(Math.max(0, Math.min(1, skill.far)) * 100);
  const beatsPersistence =
    skill.csi != null && skill.persistence_csi != null ? skill.csi > skill.persistence_csi : null;
  const windowLabel = skill.window_hours === 24 ? "today" : `over ${skill.window_hours} h`;
  const falseAlarms = falseAlarmPct == null ? "" : `, ${falseAlarmPct}% false alarms`;
  return {
    hitRatePct,
    falseAlarmPct,
    leadMinutes: skill.lead_minutes,
    windowHours: skill.window_hours,
    runs: skill.runs,
    beatsPersistence,
    sentence: `Caught ${hitRatePct}% of rain ${skill.lead_minutes} min out ${windowLabel}${falseAlarms}`,
    short: `${hitRatePct}% hit rate ${windowLabel}`,
  };
}
