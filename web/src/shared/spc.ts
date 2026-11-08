import type { Feature, FeatureCollection, Geometry, Position } from "geojson";

export type SpcKind = "watches" | "discussions";
export interface SpcProduct {
  id: string;
  number: number;
  type: "tornado" | "severe" | "md";
  issued: string;
  expires: string;
  text: string;
  probabilities: { label: string; value: string }[];
  geometry_note: string;
}

export const SPC_TYPES = [
  { type: "tornado", tag: "TOR WATCH", name: "Tornado watch", color: "#ff526b" },
  { type: "severe", tag: "SVR WATCH", name: "Severe thunderstorm watch", color: "#55aaff" },
  { type: "md", tag: "MD", name: "Mesoscale discussion", color: "#e6a5ff" },
] as const;
export const SPC_OVERLAYS = [
  { id: "watches", name: "SPC watches", hint: "Tornado and severe thunderstorm watch outlines", refreshMs: 60_000 },
  { id: "discussions", name: "Mesoscale discussions", hint: "SPC short-term weather discussions", refreshMs: 60_000 },
] as const;
export const spcStyle = (type: string) => SPC_TYPES.find((s) => s.type === type) ?? SPC_TYPES[2];
export const spcColorExpression = () => ["match", ["get", "type"], ...SPC_TYPES.flatMap((s) => [s.type, s.color]), "#e6a5ff"];
export const spcTagExpression = () => ["match", ["get", "type"], ...SPC_TYPES.flatMap((s) => [s.type, s.tag]), "SPC"];

export function activeSpc(fc: FeatureCollection, now: number): FeatureCollection {
  return { ...fc, features: (Array.isArray(fc.features) ? fc.features : []).filter((f) => Date.parse(f?.properties?.expires) > now) };
}

export function spcProduct(properties: Record<string, unknown> | null | undefined, now: number): SpcProduct | null {
  const p = properties;
  if (!p || !SPC_TYPES.some((t) => t.type === p.type) || typeof p.id !== "string" || typeof p.text !== "string"
    || typeof p.expires !== "string" || !(Date.parse(p.expires) > now)) return null;
  let probabilities = p.probabilities;
  if (typeof probabilities === "string") {
    try { probabilities = JSON.parse(probabilities); } catch { probabilities = []; }
  }
  const rows = Array.isArray(probabilities) ? probabilities.filter((r) => r && typeof r.label === "string" && typeof r.value === "string") : [];
  return { id: p.id, number: Number(p.number), type: p.type as SpcProduct["type"], issued: String(p.issued ?? ""), expires: p.expires,
    text: p.text, probabilities: rows, geometry_note: String(p.geometry_note ?? "") };
}

function inRing(point: Position, ring: Position[]): boolean {
  const [x, y] = point;
  let inside = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i], [xj, yj] = ring[j];
    const cross = (x - xi) * (yj - yi) - (y - yi) * (xj - xi);
    if ((xi !== xj || yi !== yj) && Math.abs(cross) < 1e-10 && x >= Math.min(xi, xj) && x <= Math.max(xi, xj)
      && y >= Math.min(yi, yj) && y <= Math.max(yi, yj)) return true;
    if ((yi > y) !== (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

export function containsPoint(geometry: Geometry, point: Position): boolean {
  const polygons = geometry.type === "Polygon" ? [geometry.coordinates] : geometry.type === "MultiPolygon" ? geometry.coordinates : [];
  return polygons.some(([shell, ...holes]) => shell && inRing(point, shell) && !holes.some((hole) => inRing(point, hole)));
}

const CATEGORIES = ["TSTM", "MRGL", "SLGT", "ENH", "MDT", "HIGH"];
export interface OutlookReading {
  status: "risk" | "none" | "expired" | "unavailable";
  text: string;
}

export function outlookAt(fc: FeatureCollection | null | undefined, point: Position, now: number): OutlookReading {
  if (!fc || !Array.isArray(fc.features) || !fc.features.length) return { status: "unavailable", text: "Outlook unavailable" };
  const dated = fc.features.filter((f) => Number.isFinite(Date.parse(f.properties?.expire)));
  const fresh = dated.filter((f) => Date.parse(f.properties?.expire) > now);
  if (!fresh.length) return dated.length ? { status: "expired", text: "Outlook expired" } : { status: "unavailable", text: "Outlook unavailable" };
  const known = fresh.filter((f) => CATEGORIES.includes(f.properties?.label));
  if (!known.length) return { status: "unavailable", text: "Outlook unavailable" };
  const matches: Feature[] = known.filter((f) => containsPoint(f.geometry, point));
  matches.sort((a, b) => CATEGORIES.indexOf(b.properties?.label) - CATEGORIES.indexOf(a.properties?.label));
  const p = matches[0]?.properties;
  return p ? { status: "risk", text: `${p.label}${p.name ? ` · ${p.name}` : ""}` } : { status: "none", text: "No outlined risk" };
}

export function outlookDays(collections: (FeatureCollection | null | undefined)[], point: Position, now: number): string[] {
  return [0, 1, 2].map((i) => `Day ${i + 1}: ${outlookAt(collections[i], point, now).text}`);
}
