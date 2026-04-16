FROM python:3.12-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
      curl \
    && curl -o /usr/bin/caddy -L "https://caddyserver.com/api/download?os=linux&arch=amd64" \
    && chmod +x /usr/bin/caddy \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY tile-server/api/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY tile-server/api/ /app/api/
COPY tile-server/Caddyfile /etc/caddy/Caddyfile
COPY basemap/styles/ /srv/basemap/styles/

RUN printf '#!/bin/sh\ncd /app/api && python -m uvicorn server:app --host 0.0.0.0 --port 8000 &\ncaddy run --config /etc/caddy/Caddyfile --adapter caddyfile\n' > /start.sh \
    && chmod +x /start.sh

ENV TILE_DIR=/data/tiles \
    OPEN_METEO_BASE=https://api.open-meteo.com/v1/forecast

CMD ["/bin/sh", "/start.sh"]
