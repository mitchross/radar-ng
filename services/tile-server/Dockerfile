FROM python:3.12-slim

# Install Caddy
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    && curl -o /usr/bin/caddy -L "https://caddyserver.com/api/download?os=linux&arch=amd64" \
    && chmod +x /usr/bin/caddy \
    && apt-get purge -y curl \
    && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

# Install Python deps
WORKDIR /app
COPY api/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

COPY api/ /app/api/
COPY Caddyfile /etc/caddy/Caddyfile

# Start script: FastAPI in background, Caddy in foreground
RUN printf '#!/bin/sh\ncd /app/api && python -m uvicorn server:app --host 0.0.0.0 --port 8000 &\ncaddy run --config /etc/caddy/Caddyfile --adapter caddyfile\n' > /start.sh \
    && chmod +x /start.sh

ENV TILE_DIR=/data/tiles

CMD ["/bin/sh", "/start.sh"]
