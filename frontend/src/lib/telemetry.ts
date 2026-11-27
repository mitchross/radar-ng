// Opt-in OpenTelemetry wiring for the Expo/React Native app. Exports only when
// EXPO_PUBLIC_TELEMETRY_ENABLED=1, to the build's default stack relay
// (/api/telemetry/v1/*). Location attributes are dropped before export.
//
// The OTLP/HTTP exporters use fetch() under the hood, which RN provides
// natively, no polyfills required. We don't use any auto-instrumentation
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

import Constants from "expo-constants";
import { Platform } from "react-native";
import {
  privacySafeAttributes,
  telemetryErrorType,
  type TelemetryAttributes,
} from "./telemetryPrivacy";
import { telemetryExporterConfig } from "./telemetryExport";
import { SELF_HOSTED } from "./constants";

const TELEMETRY_ENABLED = process.env.EXPO_PUBLIC_TELEMETRY_ENABLED === "1";
// A revocable abuse filter for the relay, not a secret: it ships inside the app.
const TELEMETRY_TOKEN = process.env.EXPO_PUBLIC_TELEMETRY_TOKEN as string | undefined;

// The SDK and exporters (~300 KB) are required only inside this branch.
// EXPO_PUBLIC_* values are inlined at build time, so a build without telemetry
// constant-folds the branch away and never bundles them. With it off, the API
// packages above hand out no-op tracers and loggers.
if (TELEMETRY_ENABLED) {
  /* eslint-disable @typescript-eslint/no-require-imports */
  const { resourceFromAttributes } = require("@opentelemetry/resources") as typeof import("@opentelemetry/resources");
  const { WebTracerProvider, BatchSpanProcessor } =
    require("@opentelemetry/sdk-trace-web") as typeof import("@opentelemetry/sdk-trace-web");
  const { OTLPTraceExporter } =
    require("@opentelemetry/exporter-trace-otlp-http") as typeof import("@opentelemetry/exporter-trace-otlp-http");
  const { LoggerProvider, BatchLogRecordProcessor } =
    require("@opentelemetry/sdk-logs") as typeof import("@opentelemetry/sdk-logs");
  const { OTLPLogExporter } =
    require("@opentelemetry/exporter-logs-otlp-http") as typeof import("@opentelemetry/exporter-logs-otlp-http");
  /* eslint-enable @typescript-eslint/no-require-imports */

  const resource = resourceFromAttributes({
    "service.name": "radar-ng-mobile",
    "service.version": (Constants.expoConfig?.version as string | undefined) ?? "dev",
    "deployment.environment": (process.env.EXPO_PUBLIC_ENV as string | undefined) ?? "dev",
    "device.platform": Platform.OS,
    "device.os_version": String(Platform.Version),
  });

  const tracerProvider = new WebTracerProvider({
    resource,
    spanProcessors: [
      new BatchSpanProcessor(
        new OTLPTraceExporter(
          telemetryExporterConfig(SELF_HOSTED.DEFAULT_URL, "traces", TELEMETRY_TOKEN),
        ),
        { maxExportBatchSize: 32, scheduledDelayMillis: 5000 },
      ),
    ],
  });
  tracerProvider.register();

  const loggerProvider = new LoggerProvider({
    resource,
    processors: [
      new BatchLogRecordProcessor({
        exporter: new OTLPLogExporter(
          telemetryExporterConfig(SELF_HOSTED.DEFAULT_URL, "logs", TELEMETRY_TOKEN),
        ),
        maxExportBatchSize: 32,
        scheduledDelayMillis: 5000,
      }),
    ],
  });
  logs.setGlobalLoggerProvider(loggerProvider);
}

const tracer: Tracer = otelTrace.getTracer("radar-ng-mobile");
const logger = logs.getLogger("radar-ng-mobile");

export function recordSpanError(span: Span, error: unknown): void {
  span.setAttribute("error.type", telemetryErrorType(error));
  span.setStatus({ code: SpanStatusCode.ERROR });
}

// Wrap an async operation in a span. Records exceptions and sets status
// automatically; the inner function gets the span so it can add attributes.
export async function trace<T>(
  name: string,
  fn: (span: Span) => Promise<T>,
  attributes?: TelemetryAttributes,
): Promise<T> {
  const span = tracer.startSpan(name, { attributes: privacySafeAttributes(attributes) });
  try {
    return await context.with(
      otelTrace.setSpan(context.active(), span),
      () => fn(span),
    );
  } catch (err) {
    // Free-form exception messages/stacks can contain request URLs and exact
    // coordinates. Export a bounded error type instead.
    recordSpanError(span, err);
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
  attributes?: TelemetryAttributes,
) {
  if (!TELEMETRY_ENABLED) return;
  logger.emit({
    severityNumber: severityMap[severity],
    severityText: severity.toUpperCase(),
    body,
    attributes: privacySafeAttributes(attributes),
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
if (TELEMETRY_ENABLED && errorUtils?.getGlobalHandler && errorUtils?.setGlobalHandler) {
  const prev = errorUtils.getGlobalHandler();
  errorUtils.setGlobalHandler((err, isFatal) => {
    logEvent("error", "uncaught application error", {
      "error.fatal": Boolean(isFatal),
      "error.type": telemetryErrorType(err),
    });
    prev(err, isFatal);
  });
}

export { tracer, logger };
