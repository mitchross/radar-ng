//! National Blend of Models (NBM) access for the winter forecast layers.
//!
//! The CONUS `core` files are Lambert 2.5 km (2345 x 1597) with a `.idx`. Only the 00/06/12/18z runs carry
//! 6 hour snow (`ASNOW`, metres) and ice (`FICEAC`, flat ice in mm) records on every 6th hour, so one
//! deterministic record per variable (about 0.5 MB of a 150 MB file) is fetched by byte range.
//! Rows are stored boustrophedon (scanning mode 0x50); the decoder unflips them.

use bytes::Bytes;
use chrono::{DateTime, Timelike, Utc};

use crate::decode::{MessageSelector, SURFACE_GROUND};
use crate::error::Result;
use crate::hrrr::run_id;
use crate::http::{HEAD_TIMEOUT, IDX_TIMEOUT, RANGE_TIMEOUT};
use crate::idx::{ByteRange, IdxRecord, parse_idx};
use crate::s3::S3Client;

/// NOAA NBM bucket (anonymous).
pub const BASE_URL: &str = "https://noaa-nbm-grib2-pds.s3.amazonaws.com";
/// Length of one NBM accumulation record, hours.
pub const STEP_HOURS: u32 = 6;
/// Inches per metre (`ASNOW`).
pub const IN_PER_M: f32 = 39.370_08;
/// Inches per millimetre (`FICEAC`, kg/m2 of ice).
pub const IN_PER_MM: f32 = 1.0 / 25.4;

/// One 6 hour accumulation field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NbmVar {
    /// Snowfall (0.1.29), metres.
    Asnow,
    /// Flat ice accumulation (0.1.228), mm.
    Ficeac,
}

impl NbmVar {
    /// `.idx` variable name.
    pub fn idx_name(self) -> &'static str {
        match self {
            NbmVar::Asnow => "ASNOW",
            NbmVar::Ficeac => "FICEAC",
        }
    }

    /// GRIB2 selector of the field.
    pub fn selector(self) -> MessageSelector {
        let number = match self {
            NbmVar::Asnow => 29,
            NbmVar::Ficeac => 228,
        };
        MessageSelector::parameter(0, 1, number).at_level(SURFACE_GROUND, 0.0)
    }

    /// Source value to inches.
    pub fn to_inches(self, v: f32) -> f32 {
        match self {
            NbmVar::Asnow => v * IN_PER_M,
            NbmVar::Ficeac => v * IN_PER_MM,
        }
    }
}

/// Object key of one forecast hour: `blend.YYYYMMDD/HH/core/blend.tHHz.core.fFFF.co.grib2`.
pub fn file_key(init: DateTime<Utc>, fhr: u32) -> String {
    format!(
        "blend.{}/{:02}/core/blend.t{:02}z.core.f{:03}.co.grib2",
        init.format("%Y%m%d"),
        init.hour(),
        init.hour(),
        fhr
    )
}

/// Byte range of the deterministic 6 hour record ending at `fhr`.
///
/// The line must end right after `acc fcst:`: the same prefix also starts the percentile and
/// probability records, which are not totals.
pub fn record_range(records: &[IdxRecord], var: NbmVar, fhr: u32) -> Option<ByteRange> {
    let start = fhr.checked_sub(STEP_HOURS)?;
    let tail = format!(":{}:surface:{start}-{fhr} hour acc fcst:", var.idx_name());
    let i = records.iter().position(|r| r.line.ends_with(&tail))?;
    Some(ByteRange {
        start: records[i].offset,
        end: records.get(i + 1).map(|next| next.offset - 1),
    })
}

/// Newest synoptic run whose `probe_fhr` index answers HEAD 200, as a run id `YYYYMMDD_HH`.
pub async fn find_latest_run(
    s3: &S3Client,
    now: DateTime<Utc>,
    probe_fhr: u32,
) -> Result<Option<String>> {
    for init in crate::gfs::run_candidates(now) {
        let key = format!("{}.idx", file_key(init, probe_fhr));
        if let Ok(200) = s3.head_status(&key, HEAD_TIMEOUT).await {
            return Ok(Some(run_id(init)));
        }
    }
    Ok(None)
}

/// The 6 hour records of `vars` ending at `fhr`, in `vars` order. `None` when the hour is not posted
/// yet; a variable with no record (a run without 6 hour sums) is left out.
pub async fn fetch_hour(
    s3: &S3Client,
    init: DateTime<Utc>,
    fhr: u32,
    vars: &[NbmVar],
) -> Result<Option<Vec<(NbmVar, Bytes)>>> {
    let key = file_key(init, fhr);
    let idx = match s3.get_object(&format!("{key}.idx"), IDX_TIMEOUT).await {
        Ok(bytes) => bytes,
        Err(e) if e.is_not_found() => return Ok(None),
        Err(e) => return Err(e),
    };
    let records = parse_idx(&String::from_utf8_lossy(&idx))?;
    let mut out = Vec::with_capacity(vars.len());
    for &var in vars {
        if let Some(range) = record_range(&records, var, fhr) {
            out.push((var, s3.get_range(&key, range, RANGE_TIMEOUT).await?));
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // Lines from the real blend.t12z.core.f006.co.grib2.idx of 2026-11-17.
    const IDX: &str = "\
41:47670249:d=2026111712:FICEAC:surface:0-6 hour acc fcst:
42:48189225:d=2026111712:TMP:2 m above ground:6 hour fcst:
43:48377491:d=2026111712:FICEAC:surface:0-6 hour acc@(fcst,dt=6 hour),missing=0:1% level
129:117608464:d=2026111712:ASNOW:surface:5-6 hour acc fcst:
130:118117934:d=2026111712:ASNOW:surface:5-6 hour acc fcst:prob >0.00254:prob fcst 255/255
153:129888395:d=2026111712:ASNOW:surface:0-6 hour acc fcst:
154:130422249:d=2026111712:ASNOW:surface:0-6 hour acc fcst:prob >0.00254:prob fcst 255/255
";

    #[test]
    fn picks_only_the_deterministic_six_hour_record() {
        let recs = parse_idx(IDX).unwrap();
        let snow = record_range(&recs, NbmVar::Asnow, 6).unwrap();
        assert_eq!((snow.start, snow.end), (129_888_395, Some(130_422_248)));
        let ice = record_range(&recs, NbmVar::Ficeac, 6).unwrap();
        assert_eq!((ice.start, ice.end), (47_670_249, Some(48_189_224)));
        assert!(record_range(&recs, NbmVar::Asnow, 12).is_none());
        assert!(record_range(&recs, NbmVar::Asnow, 3).is_none());
    }

    #[test]
    fn keys_use_three_digit_hours() {
        let init = Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap();
        assert_eq!(
            file_key(init, 72),
            "blend.20261117/12/core/blend.t12z.core.f072.co.grib2"
        );
    }

    #[test]
    fn units_are_inches() {
        assert!((NbmVar::Asnow.to_inches(0.0254) - 1.0).abs() < 1e-5);
        assert!((NbmVar::Ficeac.to_inches(2.54) - 0.1).abs() < 1e-6);
    }
}
