/**
 * Words for future radar frames, shared by the web radar and the app (design 3.3). HRRR's
 * dependable range is the 18 hours every run reaches: frames up to it are "Future radar", later
 * ones (HRRR extended runs and GFS) are model guidance, never "future radar". The timeline,
 * legend and inspector all read these helpers.
 */

/** Storm-scale range: every HRRR run reaches 18 h. */
export const STORM_SCALE_HORIZON_S = 18 * 3600;
export const FUTURE_RADAR = "Future radar";
export const MODEL_GUIDANCE = "Precipitation forecast · model guidance";

/** The model behind each forecast frame source. */
export const FORECAST_MODEL: Readonly<Record<string, string>> = {
  nowcast: "nowcast",
  "radar-hrrr": "HRRR",
  "radar-gfs": "GFS",
};

/** "Future radar" or model guidance for a forecast frame; `null` for observed or past frames. */
export function forecastWords(source: string, frameSec: number, nowSec: number): string | null {
  if (!(source in FORECAST_MODEL) || frameSec <= nowSec) return null;
  if (source === "radar-gfs" || frameSec - nowSec > STORM_SCALE_HORIZON_S) return MODEL_GUIDANCE;
  return FUTURE_RADAR;
}

/** The words plus the model, for the timeline line: "Future radar · HRRR". */
export function forecastLine(source: string, frameSec: number, nowSec: number): string | null {
  const words = forecastWords(source, frameSec, nowSec);
  return words && `${words} · ${FORECAST_MODEL[source]}`;
}

/** Inspector title for a forecast frame: "FUTURE RADAR" or "MODEL GUIDANCE". */
export function forecastTitle(source: string, frameSec: number, nowSec: number): string | null {
  const words = forecastWords(source, frameSec, nowSec);
  if (!words) return null;
  return words === FUTURE_RADAR ? "FUTURE RADAR" : "MODEL GUIDANCE";
}
