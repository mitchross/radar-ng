# syntax=docker/dockerfile:1.7
# radar-ng's unified Open-Meteo image.
#
# The Open-Meteo API pod and its dedicated Temporal sync worker run this same
# artifact. It starts as the worker by default; the API pod overrides the
# command with `/app/openmeteo-api serve ...`. Keeping both roles on one
# digest prevents the serving binary and sync CLI from drifting apart.
#
# This worker polls task_queue=radar-ng-open-meteo. The workflow schedules
# `open_meteo_sync` on that dedicated queue so the generic worker cannot take it.

# Pin both the release and manifest digest. A tag-only bump can silently change
# the binary or its shared-library closure (1.5.3 shipped without
# libparquet-glib and caused the 2026-07-02 outage).
FROM ghcr.io/open-meteo/open-meteo:1.5.6@sha256:4e30cdc550702e7ebe3a27d61a6640e94a5d70798e58392361dab06e6210df35

# This derivative contains radar-ng's MIT-licensed worker code and the
# AGPL-3.0-only Open-Meteo distribution. Record the immutable base explicitly;
# the release workflow repeats these annotations because metadata-action
# supplies labels at build time.
LABEL org.opencontainers.image.licenses="MIT AND AGPL-3.0-only" \
      org.opencontainers.image.base.name="ghcr.io/open-meteo/open-meteo:1.5.6" \
      org.opencontainers.image.base.digest="sha256:4e30cdc550702e7ebe3a27d61a6640e94a5d70798e58392361dab06e6210df35" \
      org.opencontainers.image.title="radar-ng-open-meteo-worker" \
      org.opencontainers.image.description="Open-Meteo API with the radar-ng Temporal sync worker" \
      org.opencontainers.image.source="https://github.com/mitchross/radar-ng"

USER root

# Add Python + minimal build chain for temporalio's grpc deps. Keep small —
# everything else lives in the upstream image already.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    for source in /etc/apt/sources.list.d/*; do \
      if [ -f "$source" ] && grep -q 'packages.apache.org/artifactory/arrow' "$source"; then \
        rm -f "$source"; \
      fi; \
    done && \
    apt-get update && apt-get install -y --no-install-recommends \
      python3 python3-pip python3-venv

# Use a venv so we don't fight the upstream image's pip layout.
RUN python3 -m venv /opt/temporal-worker-venv
ENV PATH="/opt/temporal-worker-venv/bin:$PATH"

WORKDIR /workspace

COPY temporal/requirements.txt /workspace/temporal/requirements.txt
# Install from the shared requirements file so this worker's temporalio
# stays in lockstep with the main worker's reviewed Rust Core release.
# NOTE: the previous inline form (`pip install temporalio>=1.9.0`) was
# unquoted, so the shell parsed `>=1.9.0` as an output redirection and
# installed UNPINNED latest — a temporalio 2.x release would have broken
# this image on the next build.
RUN --mount=type=cache,target=/root/.cache/pip \
    pip install -r /workspace/temporal/requirements.txt

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

# Open-Meteo resolves its data directory as ./data. Return to the upstream
# working directory so direct `sync` and `serve` invocations use /app/data;
# PYTHONPATH keeps the worker modules importable from here.
WORKDIR /app

# Switch back to the open-meteo image's non-root user (uid 999).
USER openmeteo

# Clear the upstream image's ENTRYPOINT (`./openmeteo-api`) so the default
# CMD starts Python instead of becoming arguments to the Open-Meteo CLI. The
# activity and API deployment invoke the Swift binary by absolute path.
ENTRYPOINT []
CMD ["python3", "-m", "temporal.open_meteo_worker"]
