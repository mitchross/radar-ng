# syntax=docker/dockerfile:1.7
# radar-ng open-meteo Temporal worker.
#
# Lives in a separate pod from the main radar-ng-temporal-worker because its
# base image is `ghcr.io/open-meteo/open-meteo` (Swift runtime + the
# openmeteo-api binary at /app/openmeteo-api). We layer Python + temporalio
# on top so this worker can register a single activity that subprocess-execs
# the Swift binary.
#
# This worker polls task_queue=radar-ng-open-meteo. The workflow schedules
# `open_meteo_sync` on that dedicated queue so the generic worker cannot take it.

FROM ghcr.io/open-meteo/open-meteo:latest

USER root

# Add Python + minimal build chain for temporalio's grpc deps. Keep small —
# everything else lives in the upstream image already.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-pip python3-venv

# Use a venv so we don't fight the upstream image's pip layout.
RUN python3 -m venv /opt/temporal-worker-venv
ENV PATH="/opt/temporal-worker-venv/bin:$PATH"

WORKDIR /workspace

COPY temporal/requirements.txt /workspace/temporal/requirements.txt
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install \
      temporalio>=1.9.0 \
      loguru>=0.7.2

# Copy only what the activity actually imports — this image must NOT pull
# in pygrib/pysteps/etc. The whole point of the separate pool is to keep
# these deps separated.
COPY backend/__init__.py                /workspace/backend/__init__.py
COPY backend/shared/__init__.py         /workspace/backend/shared/__init__.py
COPY backend/shared/logger.py           /workspace/backend/shared/logger.py
COPY backend/open_meteo_sync            /workspace/backend/open_meteo_sync
COPY temporal/__init__.py               /workspace/temporal/__init__.py
COPY temporal/open_meteo_worker.py      /workspace/temporal/open_meteo_worker.py

ENV PYTHONPATH=/workspace
ENV PYTHONUNBUFFERED=1

# Switch back to the open-meteo image's non-root user (uid 999).
USER openmeteo

# Clear the upstream image's ENTRYPOINT (`./openmeteo-api`) — without
# this, our CMD becomes args TO that entrypoint and the container fails
# at startup with `exec: ./openmeteo-api: stat ./openmeteo-api: no such
# file or directory` (because our WORKDIR is /workspace, not /app where
# the binary lives). The activity invokes the binary by absolute path
# (OPENMETEO_BIN env, default /app/openmeteo-api) so we don't need the
# entrypoint at all.
ENTRYPOINT []
CMD ["python3", "-m", "temporal.open_meteo_worker"]
