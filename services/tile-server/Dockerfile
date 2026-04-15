FROM python:3.12-slim AS api

WORKDIR /app/api
COPY api/requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY api/ .

FROM caddy:2-alpine

# Install Python for the API sidecar
RUN apk add --no-cache python3 py3-pip

COPY --from=api /app/api /app/api
COPY --from=api /usr/local/lib/python3.12/site-packages /usr/local/lib/python3.12/site-packages

COPY Caddyfile /etc/caddy/Caddyfile

# Start script that runs both Caddy and the FastAPI server
RUN echo '#!/bin/sh' > /start.sh && \
    echo 'cd /app/api && python -m uvicorn server:app --host 0.0.0.0 --port 8000 &' >> /start.sh && \
    echo 'caddy run --config /etc/caddy/Caddyfile --adapter caddyfile' >> /start.sh && \
    chmod +x /start.sh

ENV TILE_DIR=/data/tiles

CMD ["/start.sh"]
