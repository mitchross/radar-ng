//! WPC snow chance (`snow-chance-1`, `snow-chance-4`, `snow-chance-8`).
//!
//! The NWS Weather Prediction Center's Probabilistic Precipitation Portal posts an hourly cycle on
//! `ftp-wpc.ncep.noaa.gov` (`ftp.wpc` only redirects there) and names the newest in
//! `co/latest_cycle.txt`. Each cycle directory `co/ppp_co_24hr_YYYYMMDDHH/` holds
//! `ndfd_co_pw24ige{1p00,4p00,8p00}_YYYYMMDDHHfNNN.grib2`: the chance of at least 1, 4 or 8 in of snow
//! in the 24 h ending at hour `NNN`, one window ending every 6 h (00/06/12/18Z). GRIB2 template 4.9
//! (probability above a lower limit, in metres) of `ASNOW` on the 2.5 km NDFD Lambert grid, JPEG 2000.
//! NWS public domain. The legacy `pwpf` files end on or about 2026-11-30 and are not used.
//!
//! The stored values are fractions, 0 to 1 in steps of 0.001 (decimal scale 3, 10 bits), not percent:
//! checked on the 2026-11-18 01Z cycle and kept in the test fixture. [`chance_grid`] scales to percent.

use bytes::Bytes;
use chrono::{DateTime, NaiveDateTime, Timelike, Utc};
use radar_geo::Grid;
use radar_grib::decode::{MessageSelector, find_message};
use radar_grib::hrrr::run_id;
use radar_grib::http::{LIST_TIMEOUT, MRMS_GET_TIMEOUT};
use radar_grib::mrms::ensure_south_up;
use radar_grib::s3::S3Client;

/// PPP CONUS directory, no trailing slash.
pub const BASE_URL: &str = "https://ftp-wpc.ncep.noaa.gov/prob_precip_portal/co";
/// Names the newest complete cycle: `YYYYMMDDHH YYYYMMDDHH_MMSS`.
pub const LATEST_CYCLE_KEY: &str = "latest_cycle.txt";
/// Snow thresholds in inches, one layer each.
pub const THRESHOLDS: [u32; 3] = [1, 4, 8];
/// Window end hours outside 1..=this are ignored (the portal goes to about +74 h).
pub const MAX_FHR: u32 = 96;
/// Metres per inch (the lower limit in section 4 is in metres).
const M_PER_IN: f64 = 0.0254;

/// `ASNOW` probability in a PPP file.
pub fn selector() -> MessageSelector {
    MessageSelector::parameter(0, 1, 29)
}

fn cycle_stamp(init: DateTime<Utc>) -> String {
    init.format("%Y%m%d%H").to_string()
}

/// Directory key of a cycle, with the trailing slash the index needs.
pub fn dir_key(init: DateTime<Utc>) -> String {
    format!("ppp_co_24hr_{}/", cycle_stamp(init))
}

/// File name (no directory) of one threshold's window ending at `fhr`.
pub fn file_name(init: DateTime<Utc>, inches: u32, fhr: u32) -> String {
    format!(
        "ndfd_co_pw24ige{inches}p00_{}f{fhr:03}.grib2",
        cycle_stamp(init)
    )
}

/// Object key of one threshold's window ending at `fhr`.
pub fn file_key(init: DateTime<Utc>, inches: u32, fhr: u32) -> String {
    format!("{}{}", dir_key(init), file_name(init, inches, fhr))
}

/// The cycle named by `latest_cycle.txt` (its first word, `YYYYMMDDHH`).
pub fn parse_latest_cycle(text: &str) -> Option<DateTime<Utc>> {
    let word = text.split_whitespace().next()?;
    if word.len() != 10 {
        return None;
    }
    let t = NaiveDateTime::parse_from_str(&format!("{word}00"), "%Y%m%d%H%M").ok()?;
    Some(t.and_utc())
}

/// Window end hours the cycle's index lists for every threshold, ascending. Hours off the 6 h
/// synoptic steps or past [`MAX_FHR`] are dropped.
pub fn listed_hours(index: &str, init: DateTime<Utc>) -> Vec<u32> {
    let hours_for = |inches: u32| -> Vec<u32> {
        let prefix = format!("ndfd_co_pw24ige{inches}p00_{}f", cycle_stamp(init));
        index
            .match_indices(&prefix)
            .filter_map(|(at, _)| {
                let rest = &index[at + prefix.len()..];
                let digits = rest.get(..3)?;
                if !rest[3..].starts_with(".grib2") {
                    return None;
                }
                digits.parse::<u32>().ok()
            })
            .collect()
    };
    let mut hours = hours_for(THRESHOLDS[0]);
    for &inches in &THRESHOLDS[1..] {
        let other = hours_for(inches);
        hours.retain(|h| other.contains(h));
    }
    hours.retain(|&h| (1..=MAX_FHR).contains(&h) && (init.hour() + h).is_multiple_of(6));
    hours.sort_unstable();
    hours.dedup();
    hours
}

/// Newest cycle and its window end hours, or `None` when no cycle is posted or it lists no windows.
pub async fn find_latest_run(http: &S3Client) -> anyhow::Result<Option<(String, Vec<u32>)>> {
    let latest = match http.get_object(LATEST_CYCLE_KEY, LIST_TIMEOUT).await {
        Ok(b) => b,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let Some(init) = parse_latest_cycle(&String::from_utf8_lossy(&latest)) else {
        tracing::warn!("wpc_latest_cycle_unreadable");
        return Ok(None);
    };
    let index = match http.get_object(&dir_key(init), LIST_TIMEOUT).await {
        Ok(b) => b,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let hours = listed_hours(&String::from_utf8_lossy(&index), init);
    Ok((!hours.is_empty()).then(|| (run_id(init), hours)))
}

/// One threshold's GRIB2, or `None` when it is not posted.
pub async fn fetch(
    http: &S3Client,
    init: DateTime<Utc>,
    inches: u32,
    fhr: u32,
) -> anyhow::Result<Option<Bytes>> {
    match http
        .get_object(&file_key(init, inches, fhr), MRMS_GET_TIMEOUT)
        .await
    {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Lower limit of the first message's probability (template 4.9, "above lower limit"), in inches.
pub fn lower_limit_inches(bytes: &[u8]) -> Option<f64> {
    let mut pos = 16;
    while pos + 5 <= bytes.len() && &bytes[pos..pos + 4] != b"7777" {
        let len = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        let sec = bytes.get(pos..pos + len)?;
        if sec[4] == 4 {
            let template = u16::from_be_bytes([sec[7], sec[8]]);
            if template != 9 || *sec.get(36)? != 3 {
                return None;
            }
            let scale = i32::from(sec[37]);
            let value = u32::from_be_bytes(sec.get(38..42)?.try_into().ok()?);
            return Some(f64::from(value) * 10f64.powi(-scale) / M_PER_IN);
        }
        if len == 0 {
            return None;
        }
        pos += len;
    }
    None
}

/// The chance of at least `inches` of snow, in percent, south-up. Refuses a file for another threshold.
pub fn chance_grid(bytes: &[u8], inches: u32) -> anyhow::Result<Grid> {
    match lower_limit_inches(bytes) {
        Some(limit) if (limit - f64::from(inches)).abs() < 0.01 => {}
        other => anyhow::bail!("WPC file is not P(snow >= {inches} in): lower limit {other:?}"),
    }
    let mut msg = find_message(bytes, &selector())?
        .ok_or_else(|| anyhow::anyhow!("ASNOW probability missing from the WPC file"))?;
    ensure_south_up(&mut msg.grid);
    for v in &mut msg.grid.values {
        *v = v.clamp(0.0, 1.0) * 100.0;
    }
    Ok(msg.grid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use radar_geo::GridGeo;

    fn fixture() -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/wpc_pw24ige1p00_2026111801f035_sierra.grib2");
        std::fs::read(p).unwrap()
    }

    #[test]
    fn keys_cycle_and_listing() {
        let init = Utc.with_ymd_and_hms(2026, 11, 18, 1, 0, 0).unwrap();
        assert_eq!(
            file_key(init, 4, 35),
            "ppp_co_24hr_2026111801/ndfd_co_pw24ige4p00_2026111801f035.grib2"
        );
        assert_eq!(
            parse_latest_cycle("2026111801 2026111801_1022\n"),
            Some(init)
        );
        assert_eq!(parse_latest_cycle(""), None);
        assert_eq!(parse_latest_cycle("<html>"), None);

        // Shape of the Apache index: every file appears in an href and as link text.
        let mut index = String::new();
        for inches in THRESHOLDS {
            for fhr in [23, 29, 35] {
                let name = file_name(init, inches, fhr);
                index.push_str(&format!("<a href=\"{name}\">{name}</a>\n"));
            }
        }
        // A window missing one threshold, an off-step hour and another cycle are not listed.
        index.push_str(&format!("<a href=\"{}\">", file_name(init, 1, 41)));
        index.push_str(&format!("<a href=\"{}\">", file_name(init, 1, 24)));
        let other = Utc.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap();
        index.push_str(&format!("<a href=\"{}\">", file_name(other, 1, 47)));
        assert_eq!(listed_hours(&index, init), [23, 29, 35]);
        assert!(listed_hours("<html></html>", init).is_empty());
    }

    /// The fixture is the real `pw24ige1p00` window ending f035 of the 2026-11-18 01Z cycle, cut to the
    /// 40 x 40 cells (i 233..273, j 647..687) around its maximum in the southern Sierra Nevada and
    /// repacked as simple packing with the source's decimal scale (3). Section 4 is unchanged. The
    /// expected values were read from the full 2145 x 1377 grid at the same cells.
    #[test]
    fn real_file_stores_fractions_and_reads_as_percent() {
        let bytes = fixture();
        let raw = find_message(&bytes, &selector()).unwrap().unwrap().grid;
        let max = raw.values.iter().copied().fold(f32::MIN, f32::max);
        assert!(raw.values.iter().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(max, 1.0, "stored as a 0..1 fraction, not percent");
        assert!((lower_limit_inches(&bytes).unwrap() - 1.0).abs() < 1e-9);

        let grid = chance_grid(&bytes, 1).unwrap();
        assert_eq!((grid.width, grid.height), (40, 40));
        let GridGeo::Projected {
            lat_mesh,
            lon_mesh,
            y,
            ..
        } = &grid.geo
        else {
            panic!("NDFD is Lambert");
        };
        assert!(y[0] < y[y.len() - 1], "south-up");
        let near = |lat: f64, lon: f64| {
            let k = (0..lat_mesh.len())
                .min_by(|a, b| {
                    let d = |k: usize| (lat_mesh[k] - lat).hypot(lon_mesh[k] - lon);
                    d(*a).total_cmp(&d(*b))
                })
                .unwrap();
            grid.values[k]
        };
        for (lat, lon, pct) in [
            (36.1481, -118.2713, 100.0),
            (36.0521, -118.3070, 88.8),
            (36.1941, -117.7745, 6.6),
        ] {
            assert!((near(lat, lon) - pct).abs() < 0.01, "{lat},{lon}");
        }
        assert!(grid.values.iter().all(|v| (0.0..=100.0).contains(v)));
        assert!(
            chance_grid(&bytes, 4).is_err(),
            "a 1 in file is not the 4 in layer"
        );
    }
}
