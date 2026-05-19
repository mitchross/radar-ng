from backend.api.api.storm_watch_activities import (
    _alert_from_feature,
    _point_in_geojson,
)


def test_point_in_geojson_polygon_with_hole() -> None:
    geometry = {
        "type": "Polygon",
        "coordinates": [
            [[-90, 30], [-80, 30], [-80, 40], [-90, 40], [-90, 30]],
            [[-87, 33], [-83, 33], [-83, 37], [-87, 37], [-87, 33]],
        ],
    }

    assert _point_in_geojson(-88, 32, geometry)
    assert not _point_in_geojson(-85, 35, geometry)
    assert not _point_in_geojson(-75, 35, geometry)


def test_point_in_geojson_multipolygon() -> None:
    geometry = {
        "type": "MultiPolygon",
        "coordinates": [
            [[[-100, 30], [-95, 30], [-95, 35], [-100, 35], [-100, 30]]],
            [[[-85, 40], [-80, 40], [-80, 45], [-85, 45], [-85, 40]]],
        ],
    }

    assert _point_in_geojson(-82, 42, geometry)
    assert not _point_in_geojson(-90, 42, geometry)


def test_alert_from_feature_keeps_id_and_geometry() -> None:
    feature = {
        "id": "fallback",
        "properties": {"id": "nws-alert-1"},
        "geometry": {
            "type": "Polygon",
            "coordinates": [[[-90, 30], [-80, 30], [-80, 40], [-90, 40], [-90, 30]]],
        },
    }

    alert = _alert_from_feature(feature)

    assert alert is not None
    assert alert.alert_id == "nws-alert-1"
    assert alert.geometry["type"] == "Polygon"
