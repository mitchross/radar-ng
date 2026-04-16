FROM stormscope-base:latest

COPY shared/ /app/shared/
COPY ingest-mrms/ingest.py /app/ingest.py

ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    COLOR_TABLE_PATH=/app/shared/color_tables.json \
    POLL_INTERVAL=120

CMD ["python", "-u", "/app/ingest.py"]
