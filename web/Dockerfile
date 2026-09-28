# Static build of the web radar, served by unprivileged nginx on :8080.
# /api, /tiles and /basemap are not proxied here: the HTTPRoute for
# radar.vanillax.me sends those paths straight to the tile-server.
FROM oven/bun:1.4-alpine@sha256:d888c0ae6c86d7866ff10c5aafdd9077b36aee6455b33dd270fb93c0dd5cef6f AS build
WORKDIR /app
COPY package.json bun.lock ./
RUN bun install --frozen-lockfile
COPY . .
# The project's public phc_ key; empty builds ship with analytics off.
ARG VITE_POSTHOG_KEY=""
ENV VITE_POSTHOG_KEY=$VITE_POSTHOG_KEY
RUN bun run build

FROM nginxinc/nginx-unprivileged:1.29-alpine@sha256:0c79d56aee561a1d81c63f00eee5fb5fe29279560cdc55e91425133104c7fbe6
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 8080
