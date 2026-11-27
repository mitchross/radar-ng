// Where the app sends OTLP: the stack's own same-origin relay, never a public collector.
export type TelemetrySignal = "traces" | "logs";

export function telemetryExporterConfig(
  apiBase: string,
  signal: TelemetrySignal,
  token?: string,
): { url: string; headers: Record<string, string> } {
  const base = apiBase.replace(/\/+$/, "");
  return {
    url: `${base}/api/telemetry/v1/${signal}`,
    headers: token ? { Authorization: `Bearer ${token}` } : {},
  };
}
