# radar-ng-base

Shared Python 3.12 base image for every ingestor.

Bundles:
- eccodes (GRIB2 decoding — `libeccodes-dev`, `libeccodes-tools`)
- GDAL CLI + dev headers
- numpy, Pillow, pygrib, httpx, mercantile (see `requirements.txt`)

## Build

CI builds and pushes this image automatically via `.gitea/workflows/build-base.yml`
on any change under `services/base/**`. The pushed tag is
`registry.vanillax.me/radar-ng-base:latest`.

For local dev, from the repo root:

```sh
docker build -t registry.vanillax.me/radar-ng-base:latest \
  -f services/base/Dockerfile services/
```

Or via compose (one-shot, does not run):

```sh
docker compose -f deploy/docker-compose.yml --profile build-only build base
```

Child images reference `FROM registry.vanillax.me/radar-ng-base:latest`.
