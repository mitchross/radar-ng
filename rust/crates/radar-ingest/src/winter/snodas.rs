//! SNODAS snow depth (`snow-depth` layer), from the NSIDC G02158 masked daily tar.
//!
//! One tar a day, valid 06Z and posted about 14Z: `YYYY/MM_Mon/SNODAS_YYYYMMDD.tar`, about 8 MB. Each
//! product is a pair of gzip members, a text header and a flat big-endian int16 raster. Snow depth is
//! product code 1036, in millimetres, 6935 x 3351 cells at 30 arc-seconds, north row first, -9999 no
//! data. Citation: National Operational Hydrologic Remote Sensing Center. 2004. Snow Data Assimilation
//! System (SNODAS) Data Products at NSIDC, Version 1. doi:10.7265/N5TB14TC.

use bytes::Bytes;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, TimeZone, Utc};
use radar_geo::{Grid, GridGeo};
use radar_grib::hrrr::run_id;
use radar_grib::http::{FULL_FILE_TIMEOUT, HEAD_TIMEOUT};
use radar_grib::s3::S3Client;

/// NSIDC directory of the masked (CONUS) product, no trailing slash.
pub const BASE_URL: &str = "https://noaadata.apps.nsidc.org/NOAA/G02158/masked";
/// Hour (UTC) each daily grid is valid.
pub const VALID_HOUR: u32 = 6;
/// Days searched by [`find_latest_run`], today first.
pub const SEARCH_DAYS: i64 = 3;
/// Product code of snow depth in the member names (`us_ssmv11036tS__...`).
pub const DEPTH_CODE: &str = "11036";
/// Inches per millimetre.
pub const IN_PER_MM: f32 = 1.0 / 25.4;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Object key of one day's tar.
pub fn tar_key(date: NaiveDate) -> String {
    format!(
        "{}/{:02}_{}/SNODAS_{}.tar",
        date.year(),
        date.month(),
        MONTHS[date.month0() as usize],
        date.format("%Y%m%d")
    )
}

/// The 06Z valid times at or before `now`, newest first.
pub fn run_candidates(now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    (0..SEARCH_DAYS)
        .map(|d| {
            let date = (now - ChronoDuration::days(d)).date_naive();
            Utc.from_utc_datetime(&date.and_hms_opt(VALID_HOUR, 0, 0).expect("06:00 is valid"))
        })
        .filter(|t| *t <= now)
        .collect()
}

/// Newest day whose tar answers HEAD 200, as a run id `YYYYMMDD_06`.
pub async fn find_latest_run(
    http: &S3Client,
    now: DateTime<Utc>,
) -> anyhow::Result<Option<String>> {
    for valid in run_candidates(now) {
        if let Ok(200) = http
            .head_status(&tar_key(valid.date_naive()), HEAD_TIMEOUT)
            .await
        {
            return Ok(Some(run_id(valid)));
        }
    }
    Ok(None)
}

/// The day's tar, or `None` when it is not posted.
pub async fn fetch(http: &S3Client, valid: DateTime<Utc>) -> anyhow::Result<Option<Bytes>> {
    match http
        .get_object(&tar_key(valid.date_naive()), FULL_FILE_TIMEOUT)
        .await
    {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.is_not_found() => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Members of a ustar archive as `(name, bytes)`.
pub fn tar_members(tar: &[u8]) -> anyhow::Result<Vec<(String, &[u8])>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos + 512 <= tar.len() {
        let head = &tar[pos..pos + 512];
        if head.iter().all(|b| *b == 0) {
            break;
        }
        let field = |a: usize, b: usize| {
            let raw = &head[a..b];
            let end = raw.iter().position(|c| *c == 0).unwrap_or(raw.len());
            String::from_utf8_lossy(&raw[..end]).trim().to_string()
        };
        let size = usize::from_str_radix(&field(124, 136), 8)
            .map_err(|e| anyhow::anyhow!("bad tar size at {pos}: {e}"))?;
        let (prefix, name) = (field(345, 500), field(0, 100));
        let name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let start = pos + 512;
        let body = tar
            .get(start..start + size)
            .ok_or_else(|| anyhow::anyhow!("tar member {name} is truncated"))?;
        if head[156] == b'0' || head[156] == 0 {
            out.push((name, body));
        }
        pos = start + size.div_ceil(512) * 512;
    }
    Ok(out)
}

/// The raster layout read from a member's text header.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub cols: usize,
    pub rows: usize,
    /// Centre of the first (north-west) cell.
    pub lon0: f64,
    pub lat0: f64,
    pub dx: f64,
    pub dy: f64,
    pub nodata: f64,
    pub slope: f64,
    pub intercept: f64,
    /// Start of the valid period.
    pub valid: DateTime<Utc>,
}

/// Parse the `Key: value` header of a SNODAS member.
pub fn parse_header(text: &str) -> anyhow::Result<Header> {
    let get = |key: &str| -> anyhow::Result<&str> {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(':').map(str::trim))
            .ok_or_else(|| anyhow::anyhow!("SNODAS header lacks {key:?}"))
    };
    let num = |key: &str| -> anyhow::Result<f64> {
        get(key)?
            .parse::<f64>()
            .map_err(|e| anyhow::anyhow!("SNODAS header {key:?}: {e}"))
    };
    if get("Data type")? != "integer" || num("Data bytes per pixel")? != 2.0 {
        anyhow::bail!("SNODAS raster is not int16");
    }
    let int = |key: &str| -> anyhow::Result<u32> { Ok(num(key)? as u32) };
    let date = NaiveDate::from_ymd_opt(
        num("Start year")? as i32,
        int("Start month")?,
        int("Start day")?,
    )
    .ok_or_else(|| anyhow::anyhow!("SNODAS header has a bad start date"))?;
    let valid = date
        .and_hms_opt(int("Start hour")?, int("Start minute")?, 0)
        .ok_or_else(|| anyhow::anyhow!("SNODAS header has a bad start time"))?;
    Ok(Header {
        cols: num("Number of columns")? as usize,
        rows: num("Number of rows")? as usize,
        lon0: num("Benchmark x-axis coordinate")?,
        lat0: num("Benchmark y-axis coordinate")?,
        dx: num("X-axis resolution")?,
        dy: num("Y-axis resolution")?,
        nodata: num("No data value")?,
        slope: num("Data slope")?,
        intercept: num("Data intercept")?,
        valid: Utc.from_utc_datetime(&valid),
    })
}

/// Decode one product (`code`, e.g. [`DEPTH_CODE`]) from the tar into a south-up regular grid, with
/// each stored value times `scale` (no data is NaN). Returns the grid and its valid time.
pub fn product_grid(tar: &[u8], code: &str, scale: f32) -> anyhow::Result<(Grid, DateTime<Utc>)> {
    let members = tar_members(tar)?;
    let find = |suffix: &str| {
        members
            .iter()
            .find(|(n, _)| n.starts_with(&format!("us_ssmv{code}")) && n.ends_with(suffix))
            .map(|(_, b)| *b)
            .ok_or_else(|| anyhow::anyhow!("SNODAS tar lacks product {code} {suffix}"))
    };
    let text = radar_grib::mrms::gunzip(find(".txt.gz")?)?;
    let h = parse_header(&String::from_utf8_lossy(&text))?;
    let raw = radar_grib::mrms::gunzip(find(".dat.gz")?)?;
    if raw.len() != h.cols * h.rows * 2 {
        anyhow::bail!(
            "SNODAS raster is {} bytes, header says {} x {}",
            raw.len(),
            h.cols,
            h.rows
        );
    }
    let mut values = Vec::with_capacity(h.cols * h.rows);
    // North row first in the file; the grid is south-up.
    for r in (0..h.rows).rev() {
        let row = &raw[r * h.cols * 2..(r + 1) * h.cols * 2];
        values.extend(row.chunks_exact(2).map(|p| {
            let v = f64::from(i16::from_be_bytes([p[0], p[1]]));
            if v == h.nodata {
                f32::NAN
            } else {
                ((v * h.slope + h.intercept) as f32) * scale
            }
        }));
    }
    let lats = (0..h.rows)
        .rev()
        .map(|r| h.lat0 - r as f64 * h.dy)
        .collect();
    let lons = (0..h.cols).map(|c| h.lon0 + c as f64 * h.dx).collect();
    let grid = Grid {
        width: h.cols,
        height: h.rows,
        values,
        geo: GridGeo::Regular { lats, lons },
    };
    Ok((grid, h.valid))
}

/// Snow depth in inches.
pub fn depth_grid(tar: &[u8]) -> anyhow::Result<(Grid, DateTime<Utc>)> {
    product_grid(tar, DEPTH_CODE, IN_PER_MM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/snodas_20261117_wasatch.tar");
        std::fs::read(p).unwrap()
    }

    fn at(grid: &Grid, lat: f64, lon: f64) -> f32 {
        let GridGeo::Regular { lats, lons } = &grid.geo else {
            panic!("SNODAS is regular");
        };
        let near = |axis: &[f64], v: f64| {
            (0..axis.len())
                .min_by(|a, b| (axis[*a] - v).abs().total_cmp(&(axis[*b] - v).abs()))
                .unwrap()
        };
        grid.at(near(lats, lat), near(lons, lon))
    }

    #[test]
    fn keys_use_the_month_folder() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 17).unwrap();
        assert_eq!(tar_key(d), "2026/11_Nov/SNODAS_20261117.tar");
        let d = NaiveDate::from_ymd_opt(2027, 1, 2).unwrap();
        assert_eq!(tar_key(d), "2027/01_Jan/SNODAS_20270102.tar");
    }

    #[test]
    fn candidates_start_at_the_newest_06z() {
        let early = Utc.with_ymd_and_hms(2026, 11, 17, 5, 0, 0).unwrap();
        let ids: Vec<String> = run_candidates(early).into_iter().map(run_id).collect();
        assert_eq!(ids, ["20261116_06", "20261115_06"]);
        let late = Utc.with_ymd_and_hms(2026, 11, 17, 15, 0, 0).unwrap();
        assert_eq!(run_id(run_candidates(late)[0]), "20261117_06");
    }

    /// The fixture is the real 2026-11-17 tar cut to a 60 x 60 cell box over the Wasatch (rows
    /// 1440..1500, columns 1540..1600 of the full raster), headers rewritten to match. The expected
    /// values were read from the full 6935 x 3351 raster at the same cells.
    #[test]
    fn reads_real_snow_depth_at_known_points() {
        let tar = fixture();
        let (grid, valid) = depth_grid(&tar).unwrap();
        assert_eq!((grid.width, grid.height), (60, 60));
        assert_eq!(valid, Utc.with_ymd_and_hms(2026, 11, 17, 6, 0, 0).unwrap());
        // Deepest cell of the Wasatch box: 368 mm at 40.5458 N, 111.7292 W.
        assert!((at(&grid, 40.5458, -111.7292) - 368.0 / 25.4).abs() < 1e-3);
        // Centre cell of the box: 36 mm.
        assert!((at(&grid, 40.6208, -111.6458) - 36.0 / 25.4).abs() < 1e-3);
        // North-west corner: bare ground.
        assert_eq!(at(&grid, 40.8708, -111.8958), 0.0);
        let GridGeo::Regular { lats, .. } = &grid.geo else {
            unreachable!()
        };
        assert!(lats[0] < lats[59], "south-up");
    }

    #[test]
    fn rejects_a_tar_without_the_product() {
        let tar = fixture();
        assert!(product_grid(&tar, "11038", 1.0).is_err());
        assert!(tar_members(&tar).unwrap().len() >= 2);
    }

    #[test]
    fn header_fields_parse() {
        let text = "Data type: integer\nData bytes per pixel: 2\nNumber of columns: 6935      \n\
            Number of rows: 3351\nBenchmark x-axis coordinate: -124.729166666662\n\
            Benchmark y-axis coordinate: 52.8708333333312\nX-axis resolution: 0.008333333333333\n\
            Y-axis resolution: 0.008333333333333\nNo data value: -9999.00000000000\nData slope: 1.0\n\
            Data intercept: 0.0\nStart year: 2026\nStart month: 11\nStart day: 17\nStart hour: 6 \n\
            Start minute: 0 \n";
        let h = parse_header(text).unwrap();
        assert_eq!((h.cols, h.rows), (6935, 3351));
        assert_eq!(h.nodata, -9999.0);
        assert!(parse_header("Data type: float\n").is_err());
    }
}
