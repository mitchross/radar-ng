#!/usr/bin/env python3
"""Build rust/crates/radar-api/assets/white-christmas.json from the NCEI 1991-2020 daily normals.

Usage: build-white-christmas.py <daily_multivariate_by-station tar.gz> <out.json>

Reads each station CSV inside the tar (stdlib only, streamed) and keeps the Dec 25 value of
DLY-SNWD-PCTALL-GE001WI: the percent of years with at least 1 inch of snow on the ground.
"""

import csv
import io
import json
import sys
import tarfile

COLUMN = "DLY-SNWD-PCTALL-GE001WI"
SOURCE_URL = (
    "https://www.ncei.noaa.gov/data/normals-daily/1991-2020/archive/"
    "us-climate-normals_1991-2020_v1.0.1_daily_multivariate_by-station_c20230404.tar.gz"
)


def stations(tar_path):
    with tarfile.open(tar_path, "r:gz") as tar:
        for member in tar:
            if not member.isfile() or not member.name.endswith(".csv"):
                continue
            text = io.TextIOWrapper(tar.extractfile(member), encoding="utf-8", newline="")
            reader = csv.DictReader(text)
            if COLUMN not in (reader.fieldnames or []):
                continue
            for row in reader:
                if row["DATE"].strip() != "12-25":
                    continue
                raw = row[COLUMN].strip()
                # Missing values are blank or the -7777/-9999 sentinels.
                if not raw or float(raw) < 0:
                    break
                yield {
                    "id": row["STATION"].strip(),
                    "name": row["NAME"].strip(),
                    "lat": round(float(row["LATITUDE"]), 4),
                    "lon": round(float(row["LONGITUDE"]), 4),
                    "pct": round(float(raw), 1),
                    "years": int(row["years_" + COLUMN].strip() or 0),
                }
                break


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    rows = sorted(stations(sys.argv[1]), key=lambda r: r["id"])
    doc = {
        "source": "NOAA NCEI U.S. Climate Normals 1991-2020, daily, by station (v1.0.1, c20230404)",
        "url": SOURCE_URL,
        "column": COLUMN,
        "date": "12-25",
        "unit": "percent of years with at least 1 inch of snow on the ground",
        "license": "Public domain (U.S. Government work)",
        "stations": [[r["id"], r["name"], r["lat"], r["lon"], r["pct"], r["years"]] for r in rows],
    }
    with open(sys.argv[2], "w", encoding="utf-8") as out:
        json.dump(doc, out, separators=(",", ":"))
        out.write("\n")
    print(f"{len(rows)} stations -> {sys.argv[2]}")


if __name__ == "__main__":
    main()
