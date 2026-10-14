# radar images. One Dockerfile, three published targets, all built from the same `radar` binary:
#
#   --target api          ghcr.io/mitchross/radar-ng-tile-server      (CMD: radar api)
#   --target worker       ghcr.io/mitchross/radar-ng-temporal-worker  (CMD: radar worker; role from WORKER_ROLE)
#   --target open-meteo   ghcr.io/mitchross/radar-ng-open-meteo-worker (Open-Meteo base + radar; CMD: radar worker --role open-meteo)
#
# Each target is a drop-in for the Python image of the same name: same port, same env names, same
# uid, and the probe helpers the cluster manifests exec (`sh`, `find`, `test` from busybox).
#
# Build context is the repository root (the directory that holds rust/, backend/):
#   docker build -f deploy/rust/Dockerfile --target api -t radar-ng-radar:local .
#
# Layout the build expects inside the context:
#   rust/                     Cargo workspace (Cargo.lock committed; built with --locked)
#   backend/basemap/styles/   bundled map styles copied to /srv/basemap/styles
#   backend/basemap/fetch-glyphs.sh   glyph fetcher, pinned commit below
#
# Pins:
#   BUILDER_IMAGE    debian bookworm (glibc 2.36, same as the distroless runtime); rustup installs the
#                    toolchain pinned in rust/rust-toolchain.toml
#   RUNTIME_IMAGE    distroless cc-debian12 matches the builder's glibc; no shell, no package manager
#   busybox          static musl build, used only for container healthchecks (wget / test)
#
# Non-root: the image runs as uid 1000 (same as the Python api and worker images) so the
# pod securityContext (runAsUser 1000 / fsGroup 1000) and the Dockerfile agree.

ARG BUILDER_IMAGE=debian:bookworm-slim
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:latest
ARG BUSYBOX_IMAGE=busybox:1.36-musl
ARG ALPINE_IMAGE=alpine:3.22
ARG OPEN_METEO_IMAGE=ghcr.io/open-meteo/open-meteo:1.6.0@sha256:e1517a01a061fd96e2a9017d022c5809bb51725bcd733bcbed435e7ea32dd9da
# 1 runs the offline Temporal replay gate (every recorded Python history must replay on the Rust
# workflows) before the binary is built, like the Python image's RUN replay gate. CI sets it to 0
# only where the same gate already ran in an earlier step.
ARG REPLAY_GATE=1

# Static busybox for probes and healthchecks; distroless has no shell. The cluster's worker probes
# exec `sh -c "test $(find /tmp/worker-alive -mmin -3)"`, so sh, find and test are on PATH.
FROM ${BUSYBOX_IMAGE} AS busybox
RUN mkdir -p /out \
    && cp /bin/busybox /out/busybox \
    && for a in sh find test wget cat ls; do ln -s busybox /out/$a; done

# Glyphs for the bundled styles, fetched at build time from a pinned commit so no
# client ever loads fonts from a third party. Same pin as backend/api/Dockerfile.
FROM ${ALPINE_IMAGE} AS glyphs
RUN apk add --no-cache curl
COPY backend/basemap/fetch-glyphs.sh /fetch-glyphs.sh
RUN /fetch-glyphs.sh 028c18f713baecad011301ff7a69acc39bcc2ae7 /glyphs "Noto Sans Regular"

FROM ${BUILDER_IMAGE} AS build
# protoc: temporalio's proto build. Rust comes from rustup with the version pinned in
# rust/rust-toolchain.toml, so the image, CI and local builds use one compiler.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl gcc libc6-dev make pkg-config protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH \
    PROTOC=/usr/bin/protoc \
    CARGO_TERM_COLOR=never \
    CARGO_INCREMENTAL=0
COPY rust/rust-toolchain.toml /tmp/rust-toolchain.toml
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain "$(sed -n 's/^channel = "\(.*\)"/\1/p' /tmp/rust-toolchain.toml)"

WORKDIR /src/rust
COPY rust/ ./

# Cache mounts keep crates.io downloads and the target directory between builds. The
# binary is copied out of the (non-persisted) cache mount inside the same RUN.
ARG REPLAY_GATE
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/rust/target,sharing=locked \
    if [ "$REPLAY_GATE" = "1" ]; then \
        cargo test --release --locked -p radar-worker --test replay; \
    fi \
    && cargo build --release --locked -p radar \
    && install -D -m 0755 target/release/radar /out/radar

FROM ${RUNTIME_IMAGE} AS runtime

COPY --from=busybox /out/ /usr/local/bin/
COPY --from=build /out/radar /usr/local/bin/radar

# Bundled styles and fonts, read by the api (STYLE_DIR / BASEMAP_ROOT). Root-owned and
# world-readable, so the non-root user only reads them.
COPY backend/basemap/styles/ /srv/basemap/styles/
COPY --from=glyphs /glyphs/ /srv/basemap/fonts/

ENV LISTEN_ADDR=0.0.0.0:8080 \
    STYLE_DIR=/srv/basemap/styles \
    BASEMAP_ROOT=/srv/basemap \
    TILE_DIR=/data/tiles \
    GRID_DIR=/data/grids \
    STATE_DIR=/data/state \
    LOG_LEVEL=info

LABEL io.radar-ng.replay-gate="rust-offline-replay" \
      org.opencontainers.image.source="https://github.com/mitchross/radar-ng"

USER 1000:1000
ENTRYPOINT ["/usr/local/bin/radar"]

# Temporal worker: the role comes from WORKER_ROLE (default legacy), as with the Python image.
FROM runtime AS worker
EXPOSE 9464
CMD ["worker"]

# Open-Meteo API + sync worker, one artifact (the API pod overrides the command with
# /app/openmeteo-api serve ...). uid 999 is the upstream image's user.
FROM ${OPEN_METEO_IMAGE} AS open-meteo
LABEL org.opencontainers.image.licenses="MIT AND AGPL-3.0-only" \
      org.opencontainers.image.base.name="ghcr.io/open-meteo/open-meteo:1.6.0" \
      org.opencontainers.image.base.digest="sha256:e1517a01a061fd96e2a9017d022c5809bb51725bcd733bcbed435e7ea32dd9da" \
      org.opencontainers.image.title="radar-ng-open-meteo-worker" \
      org.opencontainers.image.description="Open-Meteo API with the radar-ng Temporal sync worker (Rust)" \
      org.opencontainers.image.source="https://github.com/mitchross/radar-ng" \
      io.radar-ng.replay-gate="rust-offline-replay"
COPY --from=busybox /out/busybox /usr/local/bin/busybox
COPY --from=build /out/radar /usr/local/bin/radar
ENV OPENMETEO_BIN=/app/openmeteo-api \
    OPENMETEO_DATA_DIR=/app/data \
    LOG_LEVEL=info
USER 999:999
ENTRYPOINT ["/usr/local/bin/radar"]
CMD ["worker", "--role", "open-meteo"]

# API (tile-server). Last stage, so a build without --target gives the api.
FROM runtime AS api
EXPOSE 8080
CMD ["api"]
