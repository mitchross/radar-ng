export interface ReplaySource { name: string; base_url: string }

export interface ReplayCase {
  id: string;
  title: string;
  summary: string;
  start: string;
  end: string;
  timezone: string;
  bounds: [number, number, number, number];
  revision: string;
  layers: string[];
  manifest_url: string;
  warnings_url: string;
  sources: ReplaySource[];
}

export interface ReplayCatalogue { schema_version: 1; cases: ReplayCase[] }

const isText = (v: unknown): v is string => typeof v === "string" && v.length > 0;

function isCase(c: any): c is ReplayCase {
  const base = `/api/replays/${c?.id}/${c?.revision}/`;
  return [c?.id, c?.title, c?.summary, c?.start, c?.end, c?.timezone, c?.revision].every(isText)
    && Array.isArray(c.bounds) && c.bounds.length === 4 && c.bounds.every(Number.isFinite)
    && Array.isArray(c.layers) && c.layers.length > 0 && c.layers.every(isText)
    && c.manifest_url === `${base}manifest.json` && c.warnings_url === `${base}warnings.geojson`
    && Array.isArray(c.sources) && c.sources.every((s: any) => isText(s?.name) && isText(s?.base_url));
}

// Throws on any shape drift so clients show an explicit error, never a live fallback.
export function parseReplayCatalogue(value: unknown): ReplayCatalogue {
  const v = value as any;
  if (v?.schema_version !== 1 || !Array.isArray(v.cases) || !v.cases.every(isCase)) {
    throw new Error("replay catalogue does not match the contract");
  }
  return v as ReplayCatalogue;
}

export function replayTileTemplate(manifest: { schema_version?: number; tile_url_template?: string }): string {
  const t = manifest.tile_url_template;
  if (manifest.schema_version !== 2 || !t?.startsWith("/replays/") || !t.includes("/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png")) {
    throw new Error("replay manifest has no case tile template");
  }
  return t;
}
