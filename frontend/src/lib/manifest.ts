import type { SelfHostedManifest } from "../types/weather";

const FRAME_KINDS = new Set(["observation", "nowcast", "model_guidance"]);
const ISO_TIMESTAMP =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?(?:Z|[+-](\d{2}):(\d{2}))$/;

export class ManifestValidationError extends Error {
  constructor(path: string) {
    super(`Invalid radar manifest at ${path}`);
    this.name = "ManifestValidationError";
  }
}

function fail(path: string): never {
  throw new ManifestValidationError(path);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isNonEmptyString(value: unknown, maxLength = 512): value is string {
  return typeof value === "string" && value.trim().length > 0 && value.length <= maxLength;
}

function isTimestamp(value: unknown): value is string {
  if (!isNonEmptyString(value)) return false;
  const match = ISO_TIMESTAMP.exec(value);
  if (!match) return false;

  const [, yearText, monthText, dayText, hourText, minuteText, secondText, offsetHourText, offsetMinuteText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  const offsetHour = Number(offsetHourText ?? 0);
  const offsetMinute = Number(offsetMinuteText ?? 0);
  if (
    year < 2000 ||
    year > 2200 ||
    month < 1 ||
    month > 12 ||
    day < 1 ||
    hour > 23 ||
    minute > 59 ||
    second > 59 ||
    offsetHour > 23 ||
    offsetMinute > 59
  ) {
    return false;
  }

  const daysInMonth = new Date(Date.UTC(year, month, 0)).getUTCDate();
  return day <= daysInMonth && Number.isFinite(Date.parse(value));
}

function validateStringArray(value: unknown, path: string): asserts value is string[] {
  if (!Array.isArray(value) || value.some((item) => !isNonEmptyString(item, 128))) {
    fail(path);
  }
}

function validateTimestampArray(value: unknown, path: string): asserts value is string[] {
  if (!Array.isArray(value) || value.some((item) => !isTimestamp(item))) {
    fail(path);
  }

  const timestamps = value as string[];
  if (new Set(timestamps).size !== timestamps.length) fail(path);
  for (let index = 1; index < timestamps.length; index += 1) {
    if (Date.parse(timestamps[index - 1]) > Date.parse(timestamps[index])) fail(path);
  }
}

function isSafeFramePath(value: unknown): value is string {
  if (!isNonEmptyString(value) || value.startsWith("/") || /[\\?#]/.test(value)) return false;
  return value.split("/").every((segment) => segment.length > 0 && segment !== "." && segment !== "..");
}

function validateOptionalNumber(
  value: unknown,
  path: string,
  predicate: (candidate: number) => boolean,
): void {
  if (value !== undefined && (typeof value !== "number" || !Number.isFinite(value) || !predicate(value))) {
    fail(path);
  }
}

function validateFrame(value: unknown, path: string): asserts value is Record<string, unknown> {
  if (!isRecord(value)) fail(path);
  if (!isTimestamp(value.timestamp)) fail(`${path}.timestamp`);
  if (!isSafeFramePath(value.path)) fail(`${path}.path`);
  if (value.source !== undefined && !isNonEmptyString(value.source, 64)) fail(`${path}.source`);
  if (value.kind !== undefined && (typeof value.kind !== "string" || !FRAME_KINDS.has(value.kind))) {
    fail(`${path}.kind`);
  }
  if (value.issued_at !== undefined && !isTimestamp(value.issued_at)) fail(`${path}.issued_at`);
  validateOptionalNumber(value.lead_minutes, `${path}.lead_minutes`, (number) => number >= 0);
  validateOptionalNumber(
    value.spatial_resolution_km,
    `${path}.spatial_resolution_km`,
    (number) => number > 0,
  );
  validateOptionalNumber(
    value.max_zoom,
    `${path}.max_zoom`,
    (number) => Number.isInteger(number) && number >= 0 && number <= 24,
  );
  if (value.palettes !== undefined) validateStringArray(value.palettes, `${path}.palettes`);
}

function validateLayer(value: unknown, path: string): void {
  if (!isRecord(value)) fail(path);
  validateTimestampArray(value.timestamps, `${path}.timestamps`);
  const timestamps = value.timestamps as string[];

  if (value.frames !== undefined) {
    if (!Array.isArray(value.frames)) fail(`${path}.frames`);
    value.frames.forEach((frame, index) => validateFrame(frame, `${path}.frames[${index}]`));

    const frameTimestamps = value.frames.map((frame) => (frame as Record<string, unknown>).timestamp);
    if (
      frameTimestamps.length !== timestamps.length ||
      frameTimestamps.some((timestamp, index) => timestamp !== timestamps[index])
    ) {
      fail(`${path}.frames`);
    }
  }

  if (value.latest !== undefined && !isTimestamp(value.latest)) fail(`${path}.latest`);
  if (value.title !== undefined && !isNonEmptyString(value.title, 256)) fail(`${path}.title`);
  if (value.kind !== undefined && !isNonEmptyString(value.kind, 64)) fail(`${path}.kind`);
  if (value.complete !== undefined && typeof value.complete !== "boolean") fail(`${path}.complete`);
}

/**
 * Validate the tile-server contract before it enters React Query or MMKV.
 * The original object is returned so additive server fields remain available
 * to newer clients without this older client having to understand them.
 */
export function parseSelfHostedManifest(value: unknown): SelfHostedManifest {
  if (!isRecord(value)) fail("root");
  if (!isRecord(value.layers)) fail("layers");
  if (!isNonEmptyString(value.tile_url_template)) fail("tile_url_template");
  if (!isTimestamp(value.updated_at)) fail("updated_at");
  if (
    value.schema_version !== undefined &&
    (typeof value.schema_version !== "number" ||
      !Number.isInteger(value.schema_version) ||
      value.schema_version < 1)
  ) {
    fail("schema_version");
  }

  for (const [layerName, layer] of Object.entries(value.layers)) {
    if (!isNonEmptyString(layerName, 64)) fail("layers");
    validateLayer(layer, `layers.${layerName}`);
  }

  return value as unknown as SelfHostedManifest;
}

export function tryParseSelfHostedManifest(value: unknown): SelfHostedManifest | undefined {
  try {
    return parseSelfHostedManifest(value);
  } catch {
    return undefined;
  }
}
