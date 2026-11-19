//! Ice on observed radar: HRRR CRAIN/CSNOW/CFRZR/CICEP become the `precip-type` grid, and MRMS
//! echoes under HRRR freezing rain or ice pellets become class 3. The fixture holds the four real
//! type records of HRRR 2026-11-17 16z f01 (valid 17z), cut from the NOAA file by `.idx` byte range.

use chrono::{DateTime, Duration, Utc};
use radar_core::grid_dump::{GridConfig, GridInput, write_grid};
use radar_core::grid_read::{GridDump, read_grid};
use radar_geo::{Grid, GridGeo};
use radar_grib::decode::find_message;
use radar_grib::hrrr::HrrrVar;
use radar_grib::mrms::ensure_south_up;
use radar_grib::units::{self, PRECIP_FREEZING_RAIN, PRECIP_ICE_PELLETS};
use radar_ingest::hrrr::PRECIP_TYPE_GRID;
use radar_ingest::hrrr::dump::{Resampler, target_axes};
use radar_ingest::precip_type::{
    MAX_ICE_GRID_DELTA, apply_hrrr_ice, classify_flags, frame_types, load_type_grid,
    nearest_type_grid,
};
use radar_tiler::TypePlane;
use radar_tiler::typed::{TYPE_ICE, TYPE_RAIN, TYPE_SNOW};

const FIXTURE: &[u8] = include_bytes!("../testdata/hrrr_types_20261117_16z_f01.grib2");
const VALID: &str = "2026-11-17T17:00:00+00:00";

fn valid() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(VALID).unwrap().to_utc()
}

/// Category per HRRR cell, as the HRRR render computes it.
fn categories() -> (Grid, Vec<f32>) {
    let msgs: Vec<_> = [
        HrrrVar::Crain,
        HrrrVar::Csnow,
        HrrrVar::Cfrzr,
        HrrrVar::Cicep,
    ]
    .into_iter()
    .map(|v| {
        let mut m = find_message(FIXTURE, &v.selector())
            .unwrap()
            .unwrap_or_else(|| panic!("{} missing from the fixture", v.key()));
        ensure_south_up(&mut m.grid);
        m
    })
    .collect();
    let template = msgs[0].grid.clone();
    let len = template.height * template.width;
    let s: Vec<Option<&[f32]>> = msgs
        .iter()
        .map(|m| Some(m.grid.values.as_slice()))
        .collect();
    let cat = units::precip_type_category(s[0], s[1], s[2], s[3], len);
    (template, cat.iter().map(|&c| c as f32).collect())
}

/// The `precip-type` grid dump, written the way `process_hour_sync` writes it.
fn write_type_grid(dir: &std::path::Path) -> GridDump {
    let (template, cat) = categories();
    let GridGeo::Projected { crs, x, y, .. } = &template.geo else {
        panic!("HRRR is projected");
    };
    let (lats, lons) = target_axes(&template, 0.03).unwrap();
    let r = Resampler::new(crs, x, y, template.height, template.width, &lats, &lons).unwrap();
    let nearest = r.resample_nearest(&cat);
    let cfg = GridConfig::with_dir(dir);
    let path = write_grid(
        &cfg,
        PRECIP_TYPE_GRID,
        VALID,
        &GridInput::new(&nearest, &lats, &lons, "category"),
    )
    .unwrap()
    .unwrap();
    let bytes = std::fs::metadata(&path).unwrap().len();
    eprintln!("precip-type grid: {bytes} bytes per valid hour");
    assert!(bytes <= 900 * 900 * 4, "{bytes}");
    read_grid(&path.with_file_name(format!("{VALID}.meta.json"))).unwrap()
}

/// First stored cell whose category is ice, as `(lat, lon)`.
fn ice_point(g: &GridDump) -> (f64, f64) {
    let m = &g.meta;
    let k = g
        .values
        .iter()
        .position(|&v| matches!(v as i32, PRECIP_FREEZING_RAIN | PRECIP_ICE_PELLETS))
        .expect("the fixture has freezing rain or ice pellets");
    let (row, col) = (k / m.width, k % m.width);
    let lat = m.lat_max - row as f64 * (m.lat_max - m.lat_min) / (m.height - 1) as f64;
    let lon = m.lon_min + col as f64 * (m.lon_max - m.lon_min) / (m.width - 1) as f64;
    (lat, lon)
}

/// A 0.01 degree MRMS-style grid (south up, 0..360 longitudes) centred on `(lat, lon)`.
fn mrms_grid(lat: f64, lon: f64, dbz: f32) -> Grid {
    let n = 21;
    let lats: Vec<f64> = (0..n).map(|i| lat - 0.1 + 0.01 * i as f64).collect();
    let lons: Vec<f64> = (0..n)
        .map(|i| lon + 360.0 - 0.1 + 0.01 * i as f64)
        .collect();
    Grid {
        height: n,
        width: n,
        values: vec![dbz; n * n],
        geo: GridGeo::Regular { lats, lons },
    }
}

#[test]
fn real_hrrr_records_have_every_category() {
    assert!(radar_core::ice::is_ice_category(
        PRECIP_FREEZING_RAIN as f32
    ));
    assert!(radar_core::ice::is_ice_category(PRECIP_ICE_PELLETS as f32));
    assert!(!radar_core::ice::is_ice_category(units::PRECIP_SNOW as f32));
    assert!(!radar_core::ice::is_ice_category(f32::NAN));
    let (_, cat) = categories();
    let count = |c: i32| cat.iter().filter(|&&v| v as i32 == c).count();
    let (rain, snow, frz, pel) = (count(1), count(2), count(3), count(4));
    assert!(rain > 1000 && snow > 1000, "rain {rain} snow {snow}");
    assert!(frz + pel > 0, "freezing rain {frz} ice pellets {pel}");
}

#[test]
fn ice_under_an_echo_becomes_class_3() {
    let tmp = tempfile::tempdir().unwrap();
    let hrrr = write_type_grid(tmp.path());
    let (lat, lon) = ice_point(&hrrr);
    let grid = mrms_grid(lat, lon, 20.0);
    let flags = TypePlane::new(vec![TYPE_SNOW; grid.values.len()]).unwrap();
    let typed = frame_types(Some(flags), &grid, Some(&hrrr)).unwrap();
    let centre = 10 * grid.width + 10;
    assert_eq!(typed.classes()[centre], TYPE_ICE);

    // Below 5 dBZ is no echo: nothing turns to ice.
    let mut quiet = mrms_grid(lat, lon, 2.0);
    quiet.values[centre] = f32::NAN;
    let mut classes = vec![TYPE_RAIN; quiet.values.len()];
    assert_eq!(apply_hrrr_ice(&mut classes, &quiet, &hrrr), 0);

    // No PrecipFlag file: ice still shows, on an all-rain base.
    let typed = frame_types(None, &grid, Some(&hrrr)).unwrap();
    assert_eq!(typed.classes()[centre], TYPE_ICE);
    assert!(
        typed
            .classes()
            .iter()
            .all(|&c| c == TYPE_RAIN || c == TYPE_ICE)
    );
}

#[test]
fn missing_or_stale_hrrr_grid_keeps_todays_classes() {
    let tmp = tempfile::tempdir().unwrap();
    // No grid at all.
    assert!(nearest_type_grid(tmp.path(), valid()).is_none());
    let hrrr = write_type_grid(tmp.path());
    let (lat, lon) = ice_point(&hrrr);
    let grid = mrms_grid(lat, lon, 20.0);
    let n = grid.values.len();
    // Today's classes: a snow band with cold rain beside it (mix).
    let flags: Vec<f32> = (0..n)
        .map(|i| if i % grid.width < 8 { 3.0 } else { 10.0 })
        .collect();
    let today = TypePlane::new(classify_flags(&flags, grid.height, grid.width, 25)).unwrap();

    // 90 minutes away is still used; one minute more is stale.
    let edge = valid() + MAX_ICE_GRID_DELTA;
    assert!(nearest_type_grid(tmp.path(), edge).is_some());
    let stale = edge + Duration::minutes(1);
    assert!(nearest_type_grid(tmp.path(), stale).is_none());
    assert!(
        nearest_type_grid(
            tmp.path(),
            valid() - MAX_ICE_GRID_DELTA - Duration::minutes(1)
        )
        .is_none()
    );
    let none = load_type_grid(tmp.path(), stale);
    assert!(none.is_none());
    assert_eq!(
        frame_types(Some(today.clone()), &grid, none.as_ref()),
        Some(today.clone())
    );
    assert_eq!(frame_types(None, &grid, None), None);

    // A grid with no ice under the frame changes nothing either.
    let far = mrms_grid(30.0, -100.0, 20.0);
    let far_today = TypePlane::new(vec![TYPE_SNOW; far.values.len()]).unwrap();
    let mut no_ice = hrrr.clone();
    no_ice.values.iter_mut().for_each(|v| *v = 1.0);
    assert_eq!(
        frame_types(Some(far_today.clone()), &far, Some(&no_ice)),
        Some(far_today)
    );
    assert_eq!(frame_types(None, &far, Some(&no_ice)), None);
}
