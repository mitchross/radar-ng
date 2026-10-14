# radar binary image: api (`radar api`), workers (`radar worker --role <role>`) and
# `radar debug`. One image serves every Rust role; the role is selected by command.
#
# Build context is the repository root (the directory that holds rust/, backend/):
#   docker build -f deploy/rust/Dockerfile -t radar-ng-radar:local .
#
# Layout the build expects inside the context:
#   rust/                     Cargo workspace (Cargo.lock committed; built with --locked)
#   backend/basemap/styles/   bundled map styles copied to /srv/basemap/styles
#   backend/basemap/fetch-glyphs.sh   glyph fetcher, pinned commit below
#
# Pins:
#   RUST_IMAGE       rust 1.92 matches workspace rust-version = "1.92" (bookworm, glibc 2.36)
#   RUNTIME_IMAGE    distroless cc-debian12 matches the builder's glibc; no shell, no package manager
#   busybox          static musl build, used only for container healthchecks (wget / test)
#
# Non-root: the image runs as uid 1000 (same as the Python api and worker images) so the
# pod securityContext (runAsUser 1000 / fsGroup 1000) and the Dockerfile agree.

ARG RUST_IMAGE=rust:1.92-bookworm
ARG RUNTIME_IMAGE=gcr.io/distroless/cc-debian12:latest
ARG BUSYBOX_IMAGE=busybox:1.36-musl
ARG ALPINE_IMAGE=alpine:3.22

# Static busybox for healthchecks; distroless has no shell or wget.
FROM ${BUSYBOX_IMAGE} AS busybox

# Glyphs for the bundled styles, fetched at build time from a pinned commit so no
# client ever loads fonts from a third party. Same pin as backend/api/Dockerfile.
FROM ${ALPINE_IMAGE} AS glyphs
RUN apk add --no-cache curl
COPY backend/basemap/fetch-glyphs.sh /fetch-glyphs.sh
RUN /fetch-glyphs.sh 028c18f713baecad011301ff7a69acc39bcc2ae7 /glyphs "Noto Sans Regular"

FROM ${RUST_IMAGE} AS build
# protoc: temporalio's proto build. PROTOC is set explicitly so the build does not depend
# on the vendored fallback resolving inside the container.
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
ENV PROTOC=/usr/bin/protoc \
    CARGO_TERM_COLOR=never \
    CARGO_INCREMENTAL=0

WORKDIR /src/rust
COPY rust/ ./

# Cache mounts keep crates.io downloads and the target directory between builds. The
# binary is copied out of the (non-persisted) cache mount inside the same RUN.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/src/rust/target,sharing=locked \
    cargo build --release --locked -p radar \
    && install -D -m 0755 target/release/radar /out/radar

FROM ${RUNTIME_IMAGE} AS runtime

COPY --from=busybox /bin/busybox /usr/local/bin/busybox
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

USER 1000:1000
EXPOSE 8080

ENTRYPOINT ["/usr/local/bin/radar"]
CMD ["api"]
