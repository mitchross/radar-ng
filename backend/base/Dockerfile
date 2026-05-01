# Shared base image for all radar-ng Python ingestors.
# Pushed by .gitea/workflows/build-base.yml as
#   registry.vanillax.me/radar-ng-base:latest
# CI retrigger: 2026-04-27 — confirm new act_runner config (gpus removed).
# Local dev build:
#   docker build -t registry.vanillax.me/radar-ng-base:latest \
#     -f base/Dockerfile backend/
#
# All ingestors FROM this image, then COPY their own ingest.py + shared/ in.

FROM python:3.12-slim

ENV DEBIAN_FRONTEND=noninteractive \
    PIP_NO_CACHE_DIR=1 \
    PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1

RUN apt-get update && apt-get install -y --no-install-recommends \
      libeccodes-dev \
      libeccodes-tools \
      gdal-bin \
      libgdal-dev \
      ca-certificates \
      curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY base/requirements.txt /tmp/base-requirements.txt
RUN pip install --no-cache-dir -r /tmp/base-requirements.txt

# Pre-create the non-root 'app' user (UID 1000) used by every child image.
# Children still run their RUN apt-get / pip steps as root (no USER set
# here), then add `USER 1000` at the end of their Dockerfile to drop privs
# for the runtime CMD. ENV HOME is set so libraries that touch ~/.cache
# (matplotlib, pyart) have a writable place to land.
RUN useradd -u 1000 -m -s /bin/sh app

# Common env defaults — overridden per-service
ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    LOG_LEVEL=INFO \
    HOME=/home/app
