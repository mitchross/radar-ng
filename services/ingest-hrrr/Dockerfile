FROM stormscope-base:latest

COPY shared/ /app/shared/
COPY ingest-hrrr/ingest.py /app/ingest.py

ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    COLOR_TABLE_PATH=/app/shared/color_tables.json \
    POLL_INTERVAL=3600 \
    FORECAST_HOURS=18 \
    EXTENDED_FORECAST_HOURS=48

CMD ["python", "-u", "/app/ingest.py"]
