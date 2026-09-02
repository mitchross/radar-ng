export type TelemetryAttributes = Record<string, string | number | boolean>;

const LOCATION_ATTRIBUTE_KEY =
  /(^|[._-])(geo|geolocation|lat|latitude|lon|lng|longitude|coord|coords|coordinate|coordinates)($|[._-])/i;
const SAFE_IDENTIFIER = /^[a-z][a-z0-9_.-]{0,63}$/i;
const SAFE_ERROR_TYPES = new Set([
  "AbortError",
  "AggregateError",
  "Error",
  "EvalError",
  "ManifestValidationError",
  "NetworkError",
  "RangeError",
  "ReferenceError",
  "SyntaxError",
  "TimeoutError",
  "TypeError",
  "URIError",
]);

/** Remove precise-location attributes even if a caller accidentally adds one. */
export function privacySafeAttributes(
  attributes?: TelemetryAttributes,
): TelemetryAttributes | undefined {
  if (!attributes) return undefined;
  return Object.fromEntries(
    Object.entries(attributes).filter(([key]) => !LOCATION_ATTRIBUTE_KEY.test(key)),
  );
}

/**
 * Query keys contain cache dimensions such as exact coordinates and server
 * URLs. Telemetry only needs the stable family at index zero.
 */
export function telemetryQueryFamily(queryKey: readonly unknown[]): string {
  const family = queryKey[0];
  return typeof family === "string" && SAFE_IDENTIFIER.test(family) ? family : "unknown";
}

/** Error names are useful for grouping without exporting free-form messages or stacks. */
export function telemetryErrorType(error: unknown): string {
  if (!(error instanceof Error)) return "NonError";
  return SAFE_ERROR_TYPES.has(error.name) ? error.name : "Error";
}
