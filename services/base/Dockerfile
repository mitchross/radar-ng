# Shared base image for all StormScope Python ingestors.
# Build from the services/ directory:
#   docker build -t stormscope-base:latest -f base/Dockerfile services/
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

# Common env defaults — overridden per-service
ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    LOG_LEVEL=INFO
