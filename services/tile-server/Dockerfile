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

# Run as a fixed non-root user so the namespace's PodSecurity restricted
# profile is satisfied (runAsNonRoot=true, runAsUser=1000). Caddy + uvicorn
# both bind ports >1024 (8080/8000) so no NET_BIND_SERVICE cap needed. We
# pre-create the dirs Caddy expects so it doesn't have to mkdir them at
# runtime against a potentially read-only path.
RUN useradd -u 1000 -m -s /bin/sh app \
    && mkdir -p /home/app/.local/share/caddy /home/app/.config/caddy \
    && chown -R app:app /app /home/app /etc/caddy
USER app
ENV HOME=/home/app

ENV TILE_DIR=/data/tiles \
    OPEN_METEO_BASE=https://api.open-meteo.com/v1/forecast

CMD ["/bin/sh", "/start.sh"]
