//! Precipitation types on forecast frames: the HRRR and GFS categorical flags become rain, snow,
//! mix and ice classes. Fixtures are real NOAA records cut by `.idx` byte range: the four HRRR
//! flags of 2026-11-17 16z f01, and the four instantaneous GFS flags of 2026-11-16 12z f019.

use radar_geo::Grid;
use radar_grib::decode::find_message;
use radar_grib::hrrr::HrrrVar;
use radar_grib::mrms::ensure_south_up;
use radar_ingest::global_model::activities::cropped_types;
use radar_ingest::global_model::state::{CONUS_BBOX, crop};
use radar_ingest::precip_type::{model_classes, plane_unless_all_rain};
use radar_tiler::typed::{TYPE_ICE, TYPE_MIX, TYPE_RAIN, TYPE_SNOW};

const HRRR: &[u8] = include_bytes!("../testdata/hrrr_types_20261117_16z_f01.grib2");
const GFS: &[u8] = include_bytes!("../testdata/gfs_types_20261116_12z_f019.grib2");

const FLAGS: [HrrrVar; 4] = [
    HrrrVar::Crain,
    HrrrVar::Csnow,
    HrrrVar::Cfrzr,
    HrrrVar::Cicep,
];

fn flag_values(bytes: &[u8]) -> (Grid, Vec<Vec<f32>>) {
    let msgs: Vec<_> = FLAGS
        .iter()
        .map(|v| {
            let mut m = find_message(bytes, &v.selector())
                .unwrap()
                .unwrap_or_else(|| panic!("{} missing from the fixture", v.key()));
            ensure_south_up(&mut m.grid);
            m
        })
        .collect();
    let template = msgs[0].grid.clone();
    (template, msgs.into_iter().map(|m| m.grid.values).collect())
}

fn counts(classes: &[u8]) -> [usize; 4] {
    let mut n = [0; 4];
    for &c in classes {
        n[usize::from(c)] += 1;
    }
    n
}

#[test]
fn model_class_rules() {
    let on = [1.0f32];
    let off = [0.0f32];
    let c = |r: &[f32], s: &[f32], z: &[f32], p: &[f32]| {
        model_classes(Some(r), Some(s), Some(z), Some(p), 1)[0]
    };
    assert_eq!(c(&on, &off, &off, &off), TYPE_RAIN);
    assert_eq!(c(&off, &on, &off, &off), TYPE_SNOW);
    assert_eq!(c(&on, &on, &off, &off), TYPE_MIX);
    assert_eq!(c(&off, &on, &on, &off), TYPE_ICE);
    assert_eq!(c(&off, &off, &off, &on), TYPE_ICE);
    assert_eq!(
        c(&off, &off, &off, &off),
        TYPE_RAIN,
        "dry cells are drawn by dBZ only"
    );
    assert_eq!(model_classes(None, None, None, None, 2), [TYPE_RAIN; 2]);
    assert!(plane_unless_all_rain(vec![TYPE_RAIN; 4]).is_none());
    assert!(plane_unless_all_rain(vec![TYPE_RAIN, TYPE_SNOW]).is_some());
}

#[test]
fn hrrr_flags_type_the_refc_cells() {
    let (grid, f) = flag_values(HRRR);
    let len = grid.height * grid.width;
    let classes = model_classes(Some(&f[0]), Some(&f[1]), Some(&f[2]), Some(&f[3]), len);
    let n = counts(&classes);
    eprintln!("hrrr classes rain/snow/mix/ice: {n:?}");
    assert_eq!(classes.len(), len);
    assert!(n[usize::from(TYPE_SNOW)] > 0, "{n:?}");
    // The #316 fixture was picked for its freezing rain and ice pellets.
    assert!(n[usize::from(TYPE_ICE)] > 0, "{n:?}");
}

#[test]
fn gfs_flags_type_the_cropped_tail() {
    let (global, _) = flag_values(GFS);
    // REFC is not in the fixture; any field on the same 0.25° grid crops the same way.
    let refc_like = crop(&global, CONUS_BBOX).unwrap();
    let plane = cropped_types(GFS, &refc_like)
        .unwrap()
        .expect("mid-November GFS has snow over CONUS");
    let n = counts(plane.classes());
    eprintln!("gfs classes rain/snow/mix/ice: {n:?}");
    assert_eq!(plane.classes().len(), refc_like.height * refc_like.width);
    assert!(n[usize::from(TYPE_SNOW)] > 0, "{n:?}");
    assert!(n[usize::from(TYPE_RAIN)] > 0, "{n:?}");
    // GFS numbers its flags 192 rain, 193 freezing rain, 194 pellets, 195 snow (the fixture's
    // records come in `.idx` order CSNOW, CICEP, CFRZR, CRAIN). Snow far outnumbers ice here.
    assert!(
        n[usize::from(TYPE_SNOW)] > 10 * n[usize::from(TYPE_ICE)],
        "{n:?}"
    );
    // A flag grid that does not match REFC is an error, not a wrong colour.
    let mut other = refc_like.clone();
    other.width -= 1;
    assert!(cropped_types(GFS, &other).is_err());
    // No flags in the bytes: no plane, the plain ramp.
    assert!(cropped_types(&[], &refc_like).unwrap().is_none());
}
