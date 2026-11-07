/**
 * Short type tags for NWS warning polygons, so colour is never the only signal on the map.
 * One table for the web and the app; the map layers read it through {@link warningTagExpression}.
 * Events not listed (advisories, statements, watches) get no tag.
 */

export const WARNING_TAGS: readonly (readonly [event: string, tag: string])[] = [
  ["Tornado Warning", "TOR"],
  ["Severe Thunderstorm Warning", "SVR"],
  ["Flash Flood Warning", "FFW"],
  ["Flood Warning", "FLW"],
  ["Special Marine Warning", "SMW"],
  ["Extreme Wind Warning", "EWW"],
  ["Snow Squall Warning", "SQW"],
  ["Dust Storm Warning", "DSW"],
];

/** Tag for an NWS event name, or "" when the type has none. */
export function warningTag(event: string | null | undefined): string {
  return WARNING_TAGS.find(([e]) => e === event)?.[1] ?? "";
}

/** MapLibre `text-field` expression: the tag of the feature's `event` property, "" (no label) otherwise. */
export function warningTagExpression(): unknown[] {
  return ["match", ["get", "event"], ...WARNING_TAGS.flat(), ""];
}
