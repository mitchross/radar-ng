# Static build of the web radar, served by static-web-server on :8080 (config: sws.toml).
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

FROM joseluisq/static-web-server:2.44.3@sha256:ccf2d666ffe5f8ef2ec96bc40f07757605998c04b78f7d8fdb98a584dd4f5717
ENV SERVER_CONFIG_FILE=/sws.toml
COPY sws.toml /sws.toml
COPY --from=build /app/dist /public
# The base image runs as root; this matches the uid the cluster Deployment sets.
USER 101:101
EXPOSE 8080
