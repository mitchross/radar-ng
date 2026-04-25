FROM registry.vanillax.me/radar-ng-base:latest

# pysteps has no cp312 prebuilt wheel on PyPI — it builds from source. Install
# gcc + headers, compile pysteps, then drop the toolchain to keep the image
# lean-ish. opencv + scipy ship binary wheels so they're fine as-is.
RUN apt-get update && apt-get install -y --no-install-recommends gcc g++ \
    && pip install --no-cache-dir \
        setuptools \
        opencv-python-headless==4.9.0.80 \
        scipy==1.12.0 \
        pysteps==1.14.0 \
    && apt-get purge -y gcc g++ \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

COPY shared/ /app/shared/
COPY nowcast/nowcast.py /app/nowcast.py

ENV TILE_DIR=/data/tiles \
    GRID_DIR=/data/grids \
    STATE_DIR=/data/state \
    PALETTES=classic \
    POLL_INTERVAL=120 \
    NOWCAST_HORIZON_MIN=60 \
    NOWCAST_STEP_MIN=5 \
    NOWCAST_INPUT_FRAMES=4

USER 1000

CMD ["python", "-u", "/app/nowcast.py"]
