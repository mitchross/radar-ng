FROM stormscope-base:latest

COPY shared/ /app/shared/
COPY ingest-tropical/ingest.py /app/ingest.py

ENV STATE_DIR=/data/state \
    POLL_INTERVAL=300

USER 1000

CMD ["python", "-u", "/app/ingest.py"]
