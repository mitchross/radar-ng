# Basemap service — protomaps/go-pmtiles with the CONUS PMTiles archive baked
# into the image so the pod needs no PVC. Rebuild when the pmtiles file is
# refreshed; serve swaps bytes on rebuild.
FROM protomaps/go-pmtiles:latest

# Upstream image is distroless (no /bin/sh, no chown), so we can't RUN
# anything. Use COPY --chown to set ownership at copy time and USER 1000
# to drop privs for runtime — that's enough to satisfy PodSecurity
# restricted (the binary only needs read on the baked-in archive).
COPY --chown=1000:1000 pmtiles-data/basemap.pmtiles /data/basemap.pmtiles
USER 1000

EXPOSE 8081

CMD ["serve", "/data", "--port", "8081", "--cors", "*"]
