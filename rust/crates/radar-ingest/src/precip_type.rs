//! Precipitation type for MRMS reflectivity frames, from MRMS `PrecipFlag_00.00` (2-minute cadence,
//! the same 0.01 degree CONUS grid as the reflectivity products).
//!
//! PrecipFlag has no freezing-rain or sleet code, so the ice class comes from the HRRR
//! `precip-type` grid valid nearest the frame (at most [`MAX_ICE_GRID_DELTA`] away).
//!
//! Best effort: any failure gives `None`, and the frame renders with the rain ramp only. A missing
//! or stale HRRR grid leaves the PrecipFlag classes exactly as they are.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use radar_core::grid_read::{GridDump, read_grid};
use radar_core::ice::is_ice_category;
pub use radar_core::ice::{ICE_ECHO_DBZ, MAX_ICE_GRID_DELTA, nearest_type_grid};
use radar_geo::{Grid, GridGeo};
use radar_grib::http::MRMS_GET_TIMEOUT;
use radar_grib::mrms as gmrms;
use radar_grib::s3::S3Client;
use radar_tiler::TypePlane;
use radar_tiler::typed::{TYPE_ICE, TYPE_MIX, TYPE_RAIN, TYPE_SNOW};
use rayon::prelude::*;

/// Default `PRECIP_FLAG_PREFIX`.
pub const PRECIP_FLAG_PREFIX: &str = "CONUS/PrecipFlag_00.00";
/// Largest gap between a reflectivity frame and the PrecipFlag file used for it. PrecipFlag lands
/// about 20 s after the reflectivity file of the same minute, so a fresh frame often takes the
/// previous file (2 minutes older).
pub const MAX_FLAG_DELTA: Duration = Duration::minutes(5);
/// Cold stratiform rain this close to snow is drawn as mix (25 cells, about 25 km).
pub const MIX_RADIUS_CELLS: usize = 25;

/// PrecipFlag code for snow (NSSL `UserTable_MRMS_PrecipFlags.csv`).
pub const FLAG_SNOW: i32 = 3;
/// PrecipFlag code for cold stratiform rain: stratiform rain with surface temperature at or below
/// 5 °C.
pub const FLAG_COLD_STRATIFORM: i32 = 10;

/// The PrecipFlag file nearest `valid_time`: `(key, gzip bytes)`, or `None` when no file is within
/// [`MAX_FLAG_DELTA`].
pub async fn fetch_flag(
    s3: &S3Client,
    prefix: &str,
    valid_time: DateTime<Utc>,
) -> anyhow::Result<Option<(String, Vec<u8>)>> {
    let keys = gmrms::list_keys_between(
        s3,
        prefix,
        valid_time - MAX_FLAG_DELTA,
        valid_time + MAX_FLAG_DELTA,
    )
    .await?;
    let Some(key) = gmrms::nearest_key(&keys, valid_time, MAX_FLAG_DELTA) else {
        return Ok(None);
    };
    let bytes = s3.get_object(key, MRMS_GET_TIMEOUT).await?;
    Ok(Some((key.to_string(), bytes.to_vec())))
}

/// Decode a PrecipFlag file and map it onto `grid`'s cells. Errors when the grids differ.
pub fn type_plane(gz: &[u8], grid: &Grid) -> anyhow::Result<TypePlane> {
    let mut flags = gmrms::decode_gz(gz)?;
    gmrms::ensure_south_up(&mut flags);
    anyhow::ensure!(
        same_cells(&flags, grid),
        "PrecipFlag grid {}x{} does not match the reflectivity grid {}x{}",
        flags.height,
        flags.width,
        grid.height,
        grid.width
    );
    Ok(TypePlane::new(classify_flags(
        &flags.values,
        flags.height,
        flags.width,
        MIX_RADIUS_CELLS,
    ))?)
}

/// The frame's type plane: PrecipFlag classes (`flags`, already decoded), with HRRR ice on top
/// when `hrrr` is given. Without PrecipFlag the base is all rain, and the plane exists only when
/// some cell is ice. Without `hrrr` this is `flags` unchanged.
pub fn frame_types(
    flags: Option<TypePlane>,
    grid: &Grid,
    hrrr: Option<&GridDump>,
) -> Option<TypePlane> {
    let Some(hrrr) = hrrr else {
        return flags;
    };
    let mut classes = match &flags {
        Some(p) => p.classes().to_vec(),
        None => vec![TYPE_RAIN; grid.values.len()],
    };
    if apply_hrrr_ice(&mut classes, grid, hrrr) == 0 {
        return flags;
    }
    TypePlane::new(classes).ok()
}

/// Read the nearest `precip-type` grid, or `None` (with a warning when one exists but is bad).
pub fn load_type_grid(grid_dir: &Path, valid_time: DateTime<Utc>) -> Option<GridDump> {
    let path = nearest_type_grid(grid_dir, valid_time)?;
    read_grid(&path)
        .inspect_err(|err| tracing::warn!(path = %path.display(), err = %err, "type_grid_failed"))
        .ok()
}

/// Set `classes` to ice where `grid` has an echo (at or above [`ICE_ECHO_DBZ`]) and
/// the nearest `hrrr` cell is freezing rain or ice pellets. Returns the number of ice cells.
pub fn apply_hrrr_ice(classes: &mut [u8], grid: &Grid, hrrr: &GridDump) -> usize {
    let values = &grid.values;
    let GridGeo::Regular { lats, lons } = &grid.geo else {
        return 0;
    };
    let m = &hrrr.meta;
    if m.height < 2
        || m.width < 2
        || classes.len() != values.len()
        || values.len() != lats.len() * lons.len()
    {
        return 0;
    }
    let dlat = (m.lat_max - m.lat_min) / (m.height - 1) as f64;
    let dlon = (m.lon_max - m.lon_min) / (m.width - 1) as f64;
    let index = |v: f64, n: usize| {
        let i = v.round();
        (i >= 0.0 && i < n as f64).then_some(i as usize)
    };
    let rows: Vec<Option<usize>> = lats
        .iter()
        .map(|&lat| index((m.lat_max - lat) / dlat, m.height))
        .collect();
    let cols: Vec<Option<usize>> = lons
        .iter()
        .map(|&lon| {
            let lon = if lon > 180.0 { lon - 360.0 } else { lon };
            index((lon - m.lon_min) / dlon, m.width)
        })
        .collect();
    let w = lons.len();
    classes
        .par_chunks_mut(w)
        .zip(values.par_chunks(w))
        .zip(rows.par_iter())
        .map(|((out, vals), row)| {
            let Some(row) = *row else {
                return 0;
            };
            let src = &hrrr.values[row * m.width..(row + 1) * m.width];
            let mut n = 0;
            for ((class, &v), col) in out.iter_mut().zip(vals).zip(&cols) {
                if v >= ICE_ECHO_DBZ
                    && let Some(col) = *col
                    && is_ice_category(src[col])
                {
                    *class = TYPE_ICE;
                    n += 1;
                }
            }
            n
        })
        .sum()
}

/// Same shape and the same first and last coordinates on both axes.
fn same_cells(a: &Grid, b: &Grid) -> bool {
    let ends = |v: &[f64]| (v.first().copied(), v.last().copied());
    let close = |x: (Option<f64>, Option<f64>), y: (Option<f64>, Option<f64>)| match (x, y) {
        ((Some(a0), Some(a1)), (Some(b0), Some(b1))) => {
            (a0 - b0).abs() < 1e-6 && (a1 - b1).abs() < 1e-6
        }
        _ => false,
    };
    match (&a.geo, &b.geo) {
        (
            GridGeo::Regular {
                lats: la,
                lons: lo_a,
            },
            GridGeo::Regular {
                lats: lb,
                lons: lo_b,
            },
        ) => {
            a.height == b.height
                && a.width == b.width
                && close(ends(la), ends(lb))
                && close(ends(lo_a), ends(lo_b))
        }
        _ => false,
    }
}

/// Display class of one PrecipFlag code, before the mix rule. Snow is snow. Every liquid code is
/// rain: warm stratiform (1), convective (6), rain mixed with hail (7, a warm-season class the
/// reflectivity ramp already shows), cold stratiform (10) and tropical (91, 96). No precipitation
/// (0), missing (-1), no coverage (-3) and unknown codes are rain too, so reflectivity alone decides
/// what draws.
pub fn class_of_flag(code: f32) -> u8 {
    if code.is_finite() && code as i32 == FLAG_SNOW {
        TYPE_SNOW
    } else {
        TYPE_RAIN
    }
}

/// Per-cell classes of a PrecipFlag grid (`h x w`, row-major). PrecipFlag has no freezing-rain or
/// sleet code, so mix is cold stratiform rain within `radius` cells of snow: the rain-snow
/// transition zone.
pub fn classify_flags(flags: &[f32], h: usize, w: usize, radius: usize) -> Vec<u8> {
    let mut classes: Vec<u8> = flags.par_iter().map(|&c| class_of_flag(c)).collect();
    if !classes.contains(&TYPE_SNOW) {
        return classes;
    }
    let near = near_mask(&classes, h, w, radius);
    classes
        .par_iter_mut()
        .zip(flags.par_iter())
        .zip(near.par_iter())
        .for_each(|((class, &code), &near)| {
            if near && code.is_finite() && code as i32 == FLAG_COLD_STRATIFORM {
                *class = TYPE_MIX;
            }
        });
    classes
}

/// `true` for every cell with a snow cell in the `(2r+1)^2` square around it.
fn near_mask(classes: &[u8], h: usize, w: usize, r: usize) -> Vec<bool> {
    // Horizontal pass per row with a running count, then a vertical running count across rows.
    let mut horiz = vec![0u8; h * w];
    horiz
        .par_chunks_mut(w)
        .zip(classes.par_chunks(w))
        .for_each(|(out, row)| {
            let snow = |c: usize| u32::from(row[c] == TYPE_SNOW);
            let mut count: u32 = (0..=r.min(w - 1)).map(snow).sum();
            for (c, dst) in out.iter_mut().enumerate() {
                *dst = u8::from(count > 0);
                if c + r + 1 < w {
                    count += snow(c + r + 1);
                }
                if c >= r {
                    count -= snow(c - r);
                }
            }
        });
    let mut near = vec![false; h * w];
    let mut count = vec![0u32; w];
    let add = |count: &mut [u32], row: usize, sign: i32| {
        for (n, &v) in count.iter_mut().zip(&horiz[row * w..(row + 1) * w]) {
            *n = n.wrapping_add_signed(sign * i32::from(v));
        }
    };
    for row in 0..=r.min(h - 1) {
        add(&mut count, row, 1);
    }
    for row in 0..h {
        for (dst, &n) in near[row * w..(row + 1) * w].iter_mut().zip(&count) {
            *dst = n > 0;
        }
        if row + r + 1 < h {
            add(&mut count, row + r + 1, 1);
        }
        if row >= r {
            add(&mut count, row - r, -1);
        }
    }
    near
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_codes_map_to_rain_and_snow() {
        for code in [
            -3.0,
            -1.0,
            0.0,
            1.0,
            6.0,
            7.0,
            10.0,
            91.0,
            96.0,
            42.0,
            f32::NAN,
        ] {
            assert_eq!(class_of_flag(code), TYPE_RAIN, "{code}");
        }
        assert_eq!(class_of_flag(3.0), TYPE_SNOW);
    }

    #[test]
    fn cold_rain_near_snow_is_mix() {
        // 1 x 8: snow at 0; cold stratiform at 1, 2 and 6; warm rain at 3.
        let flags = [3.0, 10.0, 10.0, 1.0, 0.0, 0.0, 10.0, 0.0];
        let classes = classify_flags(&flags, 1, 8, 2);
        assert_eq!(classes, [1, 2, 2, 0, 0, 0, 0, 0]);
        // Without snow nothing is mix.
        assert_eq!(classify_flags(&[10.0, 10.0], 1, 2, 2), [0, 0]);
    }

    #[test]
    fn near_mask_is_a_square_window() {
        let (h, w) = (7, 7);
        let mut classes = vec![TYPE_RAIN; h * w];
        classes[3 * w + 3] = TYPE_SNOW;
        let near = near_mask(&classes, h, w, 2);
        for r in 0..h {
            for c in 0..w {
                let inside = r.abs_diff(3) <= 2 && c.abs_diff(3) <= 2;
                assert_eq!(near[r * w + c], inside, "({r}, {c})");
            }
        }
        // A radius past the grid edge still works.
        assert!(near_mask(&classes, h, w, 20).iter().all(|&n| n));
    }
}
