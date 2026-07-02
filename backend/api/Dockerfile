# Pinned Caddy — copying the binary from the official image makes builds
# reproducible (the caddyserver.com download API always serves latest).
# Renovate picks up this tag like any other image reference.
FROM caddy:2.10.2 AS caddy

FROM python:3.12-slim

# The official image sets cap_net_bind_service=ep on the binary and COPY
# preserves xattrs — under the pod's `capabilities: drop [ALL]` that makes
# exec fail with EPERM. `install` strips xattrs; we listen on :8080 so the
# capability is unnecessary anyway.
COPY --from=caddy /usr/bin/caddy /tmp/caddy-dist
RUN install -m 0755 /tmp/caddy-dist /usr/bin/caddy && rm /tmp/caddy-dist

WORKDIR /app

# Build context is `backend/`. PYTHONPATH=/app makes `from backend...` resolve.
ENV PYTHONPATH=/app

COPY api/api/requirements.txt /app/backend/api/api/requirements.txt
RUN pip install --no-cache-dir -r /app/backend/api/api/requirements.txt

COPY shared /app/backend/shared
COPY api/api /app/backend/api/api
# Add empty __init__.py if not already present in source — required for package imports
RUN touch /app/backend/__init__.py /app/backend/shared/__init__.py /app/backend/api/__init__.py /app/backend/api/api/__init__.py

COPY api/Caddyfile /etc/caddy/Caddyfile
COPY basemap/styles/ /srv/basemap/styles/

COPY api/start.sh /start.sh
RUN chmod +x /start.sh

RUN useradd -u 1000 -m -s /bin/sh app \
    && mkdir -p /home/app/.local/share/caddy /home/app/.config/caddy \
    && chown -R app:app /app /home/app /etc/caddy
USER app
ENV HOME=/home/app

ENV TILE_DIR=/data/tiles \
    OPEN_METEO_BASE=https://api.open-meteo.com/v1/forecast \
    TEMPORAL_ADDRESS=temporal-frontend.temporal:7233 \
    TEMPORAL_NAMESPACE=default

CMD ["/bin/sh", "/start.sh"]
