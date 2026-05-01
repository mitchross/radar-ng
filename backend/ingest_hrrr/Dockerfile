FROM registry.vanillax.me/radar-ng-base:latest

COPY shared/ /app/shared/
COPY ingest_hrrr/ingest.py /app/ingest.py

ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    GRID_DIR=/data/grids \
    PALETTES=classic \
    POLL_INTERVAL=3600 \
    FORECAST_HOURS=18 \
    EXTENDED_FORECAST_HOURS=48

USER 1000

CMD ["python", "-u", "/app/ingest.py"]
