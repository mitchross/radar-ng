FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    libeccodes-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY ingest-mrms/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY shared/ /app/shared/
COPY ingest-mrms/ingest.py .

ENV TILE_DIR=/data/tiles
ENV COLOR_TABLE_PATH=/app/shared/color_tables.json
ENV POLL_INTERVAL=120

CMD ["python", "-u", "ingest.py"]
