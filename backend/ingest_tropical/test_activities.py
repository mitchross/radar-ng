"""NHC feed + GIS layers → tropical GeoJSON."""

from __future__ import annotations

from backend.ingest_tropical.activities import _build_geojson, _saffir_simpson

FEED = {
    "activeStorms": [
        {
            "id": "ep172026",
            "binNumber": "EP2",
            "name": "Polo",
            "classification": "HU",
            "intensity": "100",
            "pressure": "963",
            "latitudeNumeric": 25.9,
            "longitudeNumeric": -112.5,
            "movementDir": 35,
            "movementSpeed": 13,
            "lastUpdate": "2026-09-29T03:00:00.000Z",
            "forecastTrack": {"kmzFile": "https://example/track.kmz"},
            "publicAdvisory": {"url": "https://www.nhc.noaa.gov/text/MIATCPEP2.shtml"},
        }
    ]
}

CONE = {"features": [{"geometry": {"type": "Polygon", "coordinates": [[[0, 0], [1, 0], [1, 1], [0, 0]]]}}]}
TRACK = {"features": [{"geometry": {"type": "LineString", "coordinates": [[-112.5, 25.9], [-110, 28]]}}]}
POINTS = {
    "features": [
        {
            "geometry": {"type": "Point", "coordinates": [-110, 28]},
            "properties": {"tau": 12, "maxwind": 75, "dvlbl": "H", "datelbl": "11:00 AM Tue"},
        }
    ]
}


def _by_kind(geo, kind):
    return [f for f in geo["features"] if f["properties"]["kind"] == kind]


def test_nhc_intensity_is_knots_and_converted_to_mph():
    pos = _by_kind(_build_geojson(FEED), "position")[0]["properties"]
    assert pos["wind_kt"] == 100.0
    assert pos["wind_mph"] == 115
    assert pos["category"] == 3
    assert pos["pressure_mb"] == 963.0
    assert pos["movement_mph"] == 13.0 and pos["movement_dir_deg"] == 35.0


def test_kmz_only_feed_links_produce_no_fake_tracks():
    geo = _build_geojson(FEED)
    assert not _by_kind(geo, "track") and not _by_kind(geo, "cone")


def test_gis_layers_attach_to_the_storm_by_bin():
    geo = _build_geojson(FEED, {"EP2": {"cone": CONE, "track": TRACK, "points": POINTS}})
    assert len(_by_kind(geo, "cone")) == 1 and len(_by_kind(geo, "track")) == 1
    pt = _by_kind(geo, "forecast_point")[0]["properties"]
    assert pt["name"] == "Polo" and pt["tau_h"] == 12.0
    assert pt["wind_mph"] == 86 and pt["category"] == 1 and pt["storm_type"] == "H"


def test_gis_for_another_bin_is_ignored():
    geo = _build_geojson(FEED, {"AT1": {"cone": CONE}})
    assert not _by_kind(geo, "cone")


def test_saffir_simpson_thresholds():
    assert [_saffir_simpson(k) for k in (None, 63, 64, 83, 96, 113, 137)] == [None, None, 1, 2, 3, 4, 5]
