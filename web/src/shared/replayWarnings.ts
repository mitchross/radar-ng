export interface ReplayWarningProperties {
  year: number;
  wfo: string;
  phenomena: "TO" | "SV" | "FF";
  significance: "W";
  eventid: number;
  event: string;
  status: string;
  product_id: string;
  utc_polygon_begin: string;
  utc_polygon_end: string;
  is_emergency: boolean;
  is_pds: boolean;
}

export interface ReplayWarning {
  type: "Feature";
  id: string;
  geometry: { type: "Polygon" | "MultiPolygon"; coordinates: unknown };
  properties: ReplayWarningProperties;
}

export interface ReplayWarningHistory {
  type: "FeatureCollection";
  schema_version: 1;
  case_id: string;
  start: string;
  end: string;
  bounds: number[];
  seed: ReplayWarning[];
  features: ReplayWarning[];
  bulletins: Record<string, { url: string; sha256: string; text: string }>;
  sources: { url: string; sha256: string }[];
}

export function replayWarningsAt(history: ReplayWarningHistory, timeMs: number): ReplayWarning[] {
  if (!Number.isFinite(timeMs) || timeMs < Date.parse(history.start) || timeMs >= Date.parse(history.end)) return [];
  return history.features.filter(({ properties: p }) => p.status !== "CAN"
    && Date.parse(p.utc_polygon_begin) <= timeMs && timeMs < Date.parse(p.utc_polygon_end));
}
