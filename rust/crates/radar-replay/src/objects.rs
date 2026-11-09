//! Real MRMS object checks and per-layer coverage/gap reports (design §1).

use chrono::{DateTime, Duration, Utc};
use radar_grib::mrms as gmrms;
use radar_grib::s3::S3Client;
use serde::{Deserialize, Serialize};

use crate::case::{CaseDefinition, PRECIP_FLAG_PREFIX};

/// A run of missing expected objects between two listed objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gap {
    /// Last object before the gap.
    pub after: String,
    /// First object after the gap.
    pub before: String,
    /// Seconds between the two objects.
    pub gap_s: i64,
    /// Nominal objects missing in the gap.
    pub missing_expected: usize,
}

/// Coverage of one product in a case window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerCoverage {
    /// Layer name, or `precip-flag` for the companion product.
    pub layer: String,
    /// S3 prefix.
    pub prefix: String,
    /// Objects listed.
    pub objects: usize,
    /// First object time, when any.
    pub first: Option<String>,
    /// Last object time, when any.
    pub last: Option<String>,
    /// Objects the nominal cadence expects in the window.
    pub expected: usize,
    /// Seconds between objects used for the expectation.
    pub cadence_s: i64,
    /// Gaps larger than 1.5 cadences.
    pub gaps: Vec<Gap>,
    /// Sum of `missing_expected`.
    pub missing_expected: usize,
}

/// Full coverage report for a case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    /// Case id.
    pub case: String,
    /// Inclusive start.
    pub start: String,
    /// Exclusive end.
    pub end: String,
    /// When the check ran.
    pub checked_at: String,
    /// One entry per rendered layer.
    pub layers: Vec<LayerCoverage>,
    /// PrecipFlag companion, checked but not rendered.
    pub precip_flag: LayerCoverage,
}

/// List `prefix` objects in `[from, to)` as `(valid time, key)`, sorted.
pub async fn list_objects(
    s3: &S3Client,
    prefix: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> anyhow::Result<Vec<(DateTime<Utc>, String)>> {
    let keys = gmrms::list_keys_between(s3, prefix, from, to).await?;
    let mut out: Vec<(DateTime<Utc>, String)> = keys
        .into_iter()
        .filter_map(|k| gmrms::key_time(&k).ok().map(|t| (t, k)))
        .filter(|(t, _)| *t >= from && *t < to)
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.0 == b.0);
    Ok(out)
}

/// Build a coverage report from listed objects.
pub fn coverage_for(
    layer: &str,
    prefix: &str,
    cadence_s: i64,
    expected: usize,
    objects: &[(DateTime<Utc>, String)],
) -> LayerCoverage {
    let limit = Duration::seconds(cadence_s + cadence_s / 2);
    let mut gaps = Vec::new();
    let mut missing = 0usize;
    for pair in objects.windows(2) {
        let gap_s = (pair[1].0 - pair[0].0).num_seconds();
        if gap_s > limit.num_seconds() {
            let n = ((gap_s + cadence_s / 2) / cadence_s - 1).max(0) as usize;
            missing += n;
            gaps.push(Gap {
                after: gmrms::format_valid_time(pair[0].0),
                before: gmrms::format_valid_time(pair[1].0),
                gap_s,
                missing_expected: n,
            });
        }
    }
    LayerCoverage {
        layer: layer.to_string(),
        prefix: prefix.to_string(),
        objects: objects.len(),
        first: objects.first().map(|(t, _)| gmrms::format_valid_time(*t)),
        last: objects.last().map(|(t, _)| gmrms::format_valid_time(*t)),
        expected,
        cadence_s,
        gaps,
        missing_expected: missing,
    }
}

/// Expected objects in a half-open window for a nominal cadence.
pub fn expected_count(from: DateTime<Utc>, to: DateTime<Utc>, cadence_s: i64) -> usize {
    let secs = (to - from).num_seconds().max(0);
    (secs / cadence_s + 1) as usize
}

/// Check every product of `case` against the live bucket.
pub async fn check_case(s3: &S3Client, case_id: &str) -> anyhow::Result<Coverage> {
    let case = crate::case::find(case_id).map_err(|e| anyhow::anyhow!("{e}"))?;
    let (from, to) = case.window().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut layers = Vec::new();
    for spec in case.layers() {
        let objects = list_objects(s3, spec.prefix, from, to).await?;
        layers.push(coverage_for(
            spec.layer,
            spec.prefix,
            spec.cadence_s,
            expected_count(from, to, spec.cadence_s),
            &objects,
        ));
    }
    let flag = list_objects(s3, PRECIP_FLAG_PREFIX, from, to).await?;
    let precip_flag = coverage_for(
        "precip-flag",
        PRECIP_FLAG_PREFIX,
        120,
        expected_count(from, to, 120),
        &flag,
    );
    Ok(Coverage {
        case: case.id.to_string(),
        start: case.start.to_string(),
        end: case.end.to_string(),
        checked_at: gmrms::format_valid_time(Utc::now()),
        layers,
        precip_flag,
    })
}

/// Every layer's listed objects, in one pass, for the importer.
pub async fn list_case_objects(
    s3: &S3Client,
    case: &CaseDefinition,
) -> anyhow::Result<Vec<(&'static crate::case::LayerSpec, Vec<(DateTime<Utc>, String)>)>> {
    let (from, to) = case.window().map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut out = Vec::new();
    for spec in case.layers() {
        let objects = list_objects(s3, spec.prefix, from, to).await?;
        out.push((spec, objects));
    }
    Ok(out)
}
