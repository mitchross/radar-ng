export interface RadarNowcastPoint {
  timestamp: string;
  lead_minutes: number | null;
  dbz: number | null;
  precipitation_mm_h: number;
}

/**
 * How the nowcast has actually done over the server's rolling window, scored
 * against the radar that arrived later at one lead time. Fractions 0–1; null
 * when the denominator is empty (no rain observed, no rain forecast).
 */
export interface NowcastSkillHeadline {
  lead_minutes: number;
  window_hours: number;
  /** Distinct nowcast runs behind the numbers. */
  runs: number;
  /** Probability of detection: share of observed rain the forecast caught. */
  pod: number | null;
  /** False alarm ratio: share of forecast rain that never arrived. */
  far: number | null;
  csi: number | null;
  /** CSI of a "nothing moves" forecast from the same start, for comparison. */
  persistence_csi: number | null;
}

export interface RadarNowcastResponse {
  status: "ok" | "degraded" | "unavailable";
  source?: "mrms-nowcast";
  method?: string;
  issued_at?: string;
  horizon_minutes?: number;
  step_minutes?: number;
  spatial_resolution_km?: number;
  latitude?: number;
  longitude?: number;
  reason?: string;
  detail?: string | null;
  points: RadarNowcastPoint[];
  skill?: NowcastSkillHeadline | null;
}

