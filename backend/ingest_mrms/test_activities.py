import numpy as np

from backend.ingest_mrms.activities import _grid_axes


class _Msg(dict):
    def latlons(self):
        lats = np.array([[30.0, 30.0], [29.0, 29.0]])
        lons = np.array([[250.0, 251.0], [250.0, 251.0]])
        return lats, lons


def test_regular_grid_axes_come_from_the_header_without_meshes():
    msg = _Msg(
        gridType="regular_ll", Nj=3500, Ni=7000,
        latitudeOfFirstGridPointInDegrees=54.995, latitudeOfLastGridPointInDegrees=20.005001,
        longitudeOfFirstGridPointInDegrees=230.005, longitudeOfLastGridPointInDegrees=299.994998,
    )
    msg.latlons = None  # must not be called for regular grids
    lat, lon = _grid_axes(msg)
    assert lat.shape == (3500,) and lon.shape == (7000,)
    assert lat[0] == 54.995 and lat[-1] == 20.005001
    assert np.allclose(np.diff(lat), -0.01, atol=1e-6) and np.allclose(np.diff(lon), 0.01, atol=1e-6)


def test_other_grids_fall_back_to_latlons():
    lat, lon = _grid_axes(_Msg(gridType="lambert"))
    assert lat.tolist() == [30.0, 29.0] and lon.tolist() == [250.0, 251.0]
