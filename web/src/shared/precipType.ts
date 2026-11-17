/**
 * Precipitation-type wording shared by the web and the app. `/api/inspect/radar…` answers
 * `"precip_class": "ice"` where observed radar is drawn in the ice class (HRRR model guidance).
 */
export const ICE_NOTE = "Ice: freezing rain or sleet, from HRRR model guidance";

/** The inspector note for an `/api/inspect` body, or null. */
export function precipClassNote(body: { precip_class?: unknown } | null | undefined): string | null {
  return body?.precip_class === "ice" ? ICE_NOTE : null;
}
