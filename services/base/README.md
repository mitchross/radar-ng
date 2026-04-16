# stormscope-base

Shared Python 3.12 base image for every ingestor.

Bundles:
- eccodes (GRIB2 decoding — `libeccodes-dev`, `libeccodes-tools`)
- GDAL CLI + dev headers
- numpy, Pillow, pygrib, httpx, mercantile (see `requirements.txt`)

## Build

From the repo root:

```sh
docker build -t stormscope-base:latest -f services/base/Dockerfile services/
```

Or via compose (one-shot, does not run):

```sh
docker compose -f deploy/docker-compose.yml --profile build-only build base
```

Child images reference `FROM stormscope-base:latest`.
