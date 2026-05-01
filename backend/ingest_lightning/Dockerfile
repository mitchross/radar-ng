FROM registry.vanillax.me/radar-ng-base:latest

RUN pip install --no-cache-dir websockets==12.0

COPY shared/ /app/shared/
COPY ingest_lightning/ingest.py /app/ingest.py

ENV STATE_DIR=/data/state \
    LIGHTNING_RETENTION_MIN=15 \
    LIGHTNING_FLUSH_S=2.0 \
    LIGHTNING_MAX_STRIKES=5000 \
    LIGHTNING_LAT_MIN=15 \
    LIGHTNING_LAT_MAX=55 \
    LIGHTNING_LON_MIN=-130 \
    LIGHTNING_LON_MAX=-50

USER 1000

CMD ["python", "-u", "/app/ingest.py"]
