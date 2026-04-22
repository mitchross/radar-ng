# Basemap service — protomaps/go-pmtiles with the CONUS PMTiles archive baked
# into the image so the pod needs no PVC. Rebuild when the pmtiles file is
# refreshed; serve swaps bytes on rebuild.
FROM protomaps/go-pmtiles:latest

COPY pmtiles-data/basemap.pmtiles /data/basemap.pmtiles

EXPOSE 8081

CMD ["serve", "/data", "--port", "8081", "--cors", "*"]
