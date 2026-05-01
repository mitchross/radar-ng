"""OTEL bootstrap for the Temporal worker.

Configures the OTLP exporter from `OTEL_EXPORTER_OTLP_ENDPOINT` and returns
the `TracingInterceptor` that the worker registers so every workflow + every
activity emits a span.
"""

from __future__ import annotations

import os

from opentelemetry import trace
from opentelemetry.sdk.resources import Resource
from opentelemetry.sdk.trace import TracerProvider
from opentelemetry.sdk.trace.export import BatchSpanProcessor
from opentelemetry.exporter.otlp.proto.grpc.trace_exporter import OTLPSpanExporter
from temporalio.contrib.opentelemetry import TracingInterceptor


SERVICE_NAME = "radar-ng-temporal-worker"


def init_tracer() -> TracingInterceptor:
    endpoint = os.environ.get("OTEL_EXPORTER_OTLP_ENDPOINT")
    resource = Resource.create({"service.name": SERVICE_NAME})
    provider = TracerProvider(resource=resource)
    if endpoint:
        provider.add_span_processor(BatchSpanProcessor(OTLPSpanExporter(endpoint=endpoint)))
    trace.set_tracer_provider(provider)
    return TracingInterceptor()
