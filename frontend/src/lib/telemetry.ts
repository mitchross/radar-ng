// OpenTelemetry wiring for the Expo/React Native app. Ships traces + logs
// to the self-hosted OTEL Gateway at otel.vanillax.me, which fans out to
// Tempo (traces) and Loki (logs). View in Grafana at grafana.vanillax.me
// (datasources: Tempo for traces, Loki with {service_name="radar-ng-mobile"}
// for logs).
//
// The OTLP/HTTP exporters use fetch() under the hood, which RN provides
// natively — no polyfills required. We don't use any auto-instrumentation
// that depends on DOM (PerformanceObserver, window, etc.), so this is
// entirely manual tracing via trace() / logEvent() helpers below.

import {
  trace as otelTrace,
  context,
  SpanStatusCode,
  Span,
  Tracer,
} from "@opentelemetry/api";
import { logs, SeverityNumber } from "@opentelemetry/api-logs";
import { resourceFromAttributes } from "@opentelemetry/resources";
import {
  ATTR_SERVICE_NAME,
  ATTR_SERVICE_VERSION,
} from "@opentelemetry/semantic-conventions";
import {
  WebTracerProvider,
  BatchSpanProcessor,
} from "@opentelemetry/sdk-trace-web";
import { OTLPTraceExporter } from "@opentelemetry/exporter-trace-otlp-http";
import {
  LoggerProvider,
  BatchLogRecordProcessor,
} from "@opentelemetry/sdk-logs";
import { OTLPLogExporter } from "@opentelemetry/exporter-logs-otlp-http";

import Constants from "expo-constants";
import { Platform } from "react-native";

const OTLP_BASE =
  (process.env.EXPO_PUBLIC_OTLP_BASE as string | undefined) ??
  "https://otel.vanillax.me";

const resource = resourceFromAttributes({
  [ATTR_SERVICE_NAME]: "radar-ng-mobile",
  [ATTR_SERVICE_VERSION]:
    (Constants.expoConfig?.version as string | undefined) ?? "dev",
  "deployment.environment":
    (process.env.EXPO_PUBLIC_ENV as string | undefined) ?? "dev",
  "device.platform": Platform.OS,
  "device.os_version": String(Platform.Version),
});

const tracerProvider = new WebTracerProvider({
  resource,
  spanProcessors: [
    new BatchSpanProcessor(
      new OTLPTraceExporter({ url: `${OTLP_BASE}/v1/traces` }),
      { maxExportBatchSize: 32, scheduledDelayMillis: 5000 },
    ),
  ],
});
tracerProvider.register();

const loggerProvider = new LoggerProvider({
  resource,
  processors: [
    new BatchLogRecordProcessor(
      new OTLPLogExporter({ url: `${OTLP_BASE}/v1/logs` }),
      { maxExportBatchSize: 32, scheduledDelayMillis: 5000 },
    ),
  ],
});
logs.setGlobalLoggerProvider(loggerProvider);

const tracer: Tracer = otelTrace.getTracer("radar-ng-mobile");
const logger = logs.getLogger("radar-ng-mobile");

// Wrap an async operation in a span. Records exceptions and sets status
// automatically; the inner function gets the span so it can add attributes.
export async function trace<T>(
  name: string,
  fn: (span: Span) => Promise<T>,
  attributes?: Record<string, string | number | boolean>,
): Promise<T> {
  const span = tracer.startSpan(name, { attributes });
  try {
    return await context.with(
      otelTrace.setSpan(context.active(), span),
      () => fn(span),
    );
  } catch (err) {
    span.recordException(err as Error);
    span.setStatus({
      code: SpanStatusCode.ERROR,
      message: (err as Error).message,
    });
    throw err;
  } finally {
    span.end();
  }
}

type Severity = "debug" | "info" | "warn" | "error";

const severityMap: Record<Severity, SeverityNumber> = {
  debug: SeverityNumber.DEBUG,
  info: SeverityNumber.INFO,
  warn: SeverityNumber.WARN,
  error: SeverityNumber.ERROR,
};

export function logEvent(
  severity: Severity,
  body: string,
  attributes?: Record<string, string | number | boolean>,
) {
  logger.emit({
    severityNumber: severityMap[severity],
    severityText: severity.toUpperCase(),
    body,
    attributes,
  });
}

// Pipe React Native's global JS error handler into the OTEL log stream so
// red-screen crashes and unhandled promise rejections show up in Loki.
// Keeps the original handler chain intact so the dev red-screen still pops.
type GlobalErrorUtils = {
  getGlobalHandler?: () => (err: unknown, isFatal?: boolean) => void;
  setGlobalHandler?: (
    handler: (err: unknown, isFatal?: boolean) => void,
  ) => void;
};
const errorUtils = (globalThis as unknown as { ErrorUtils?: GlobalErrorUtils })
  .ErrorUtils;
if (errorUtils?.getGlobalHandler && errorUtils?.setGlobalHandler) {
  const prev = errorUtils.getGlobalHandler();
  errorUtils.setGlobalHandler((err, isFatal) => {
    const e = err as Error;
    logEvent("error", e?.message ?? String(err), {
      "error.fatal": Boolean(isFatal),
      "error.stack": e?.stack ?? "",
      "error.name": e?.name ?? "Error",
    });
    prev(err, isFatal);
  });
}

export { tracer, logger };
