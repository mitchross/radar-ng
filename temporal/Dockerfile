# radar-ng Temporal worker
#
# Single image containing every workflow + every activity registered on
# task queue `radar-ng`. Runs as a TemporalWorkerDeployment CR in Talos.
#
# Build context: repo root (so we can COPY backend/ + temporal/ together).
#   docker build -f temporal/Dockerfile -t radar-ng-temporal-worker .
FROM python:3.12-slim

ENV PYTHONUNBUFFERED=1 \
    PYTHONDONTWRITEBYTECODE=1 \
    PIP_NO_CACHE_DIR=1

# System deps for scientific Python (cfgrib/eccodes, gdal, pyproj)
RUN apt-get update && apt-get install -y --no-install-recommends \
      libeccodes-dev libgdal-dev libproj-dev \
      build-essential gcc g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Worker deps first for layer caching
COPY temporal/requirements.txt /app/temporal/requirements.txt
RUN pip install -r /app/temporal/requirements.txt

# Activity modules live in backend/ and import their own deps. Each backend
# service ships its own requirements.txt; install them all into this image
# so the worker can call any activity.
COPY backend/shared /app/backend/shared
COPY backend/api/api/requirements.txt /app/backend/api/requirements.txt
COPY backend/ingest_mrms/requirements.txt /app/backend/ingest_mrms/requirements.txt
COPY backend/ingest_hrrr/requirements.txt /app/backend/ingest_hrrr/requirements.txt
COPY backend/base/requirements.txt /app/backend/base/requirements.txt
RUN pip install \
      -r /app/backend/base/requirements.txt \
      -r /app/backend/ingest_mrms/requirements.txt \
      -r /app/backend/ingest_hrrr/requirements.txt

# Copy source last so code changes don't bust dep cache
COPY backend /app/backend
COPY temporal /app/temporal

# Treat /app as the import root so `from backend...` and `from temporal...` work
ENV PYTHONPATH=/app

CMD ["python", "-m", "temporal.worker"]
