FROM registry.vanillax.me/radar-ng-base:latest

COPY shared/ /app/shared/
COPY ingest_mrms/ingest.py /app/ingest.py

ENV TILE_DIR=/data/tiles \
    STATE_DIR=/data/state \
    GRID_DIR=/data/grids \
    PALETTES=classic \
    POLL_INTERVAL=120

USER 1000

CMD ["python", "-u", "/app/ingest.py"]
