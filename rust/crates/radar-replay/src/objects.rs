//! Real MRMS object checks and per-layer coverage/gap reports (design §1).

use chrono::{DateTime, Duration, Utc};
use radar_grib::mrms as gmrms;
use radar_grib::s3::S3Client;
use serde::{Deserialize, Serialize};

use crate::case::PRECIP_FLAG_PREFIX;

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
    /// Exact archive keys retained for reproducibility.
    pub keys: Vec<String>,
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
    let listed = keys
        .into_iter()
        .map(|key| Ok((gmrms::key_time(&key)?, key)))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut out: Vec<(DateTime<Utc>, String)> = listed
        .into_iter()
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
    from: DateTime<Utc>,
    to: DateTime<Utc>,
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
    let expected = expected_count(from, to, cadence_s);
    let edges = match (objects.first(), objects.last()) {
        (Some(first), Some(last)) => vec![(from, first.0), (last.0, to)],
        _ => vec![(from, to)],
    };
    for (a, b) in edges {
        let gap_s = (b - a).num_seconds();
        let n = (gap_s / cadence_s).max(0) as usize;
        if n > 0 {
            missing += n;
            gaps.push(Gap {
                after: gmrms::format_valid_time(a),
                before: gmrms::format_valid_time(b),
                gap_s,
                missing_expected: n,
            });
        }
    }
    LayerCoverage {
        layer: layer.to_string(),
        prefix: prefix.to_string(),
        objects: objects.len(),
        keys: objects.iter().map(|(_, key)| key.clone()).collect(),
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
    ((secs + cadence_s - 1) / cadence_s) as usize
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
            from,
            to,
            &objects,
        ));
    }
    let flag = list_objects(s3, PRECIP_FLAG_PREFIX, from, to).await?;
    let precip_flag = coverage_for("precip-flag", PRECIP_FLAG_PREFIX, 120, from, to, &flag);
    Ok(Coverage {
        case: case.id.to_string(),
        start: case.start.to_string(),
        end: case.end.to_string(),
        checked_at: gmrms::format_valid_time(Utc::now()),
        layers,
        precip_flag,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_open_counts_and_missing_edges_are_recorded() {
        let from = DateTime::parse_from_rfc3339("2023-03-24T23:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = from + Duration::hours(4);
        assert_eq!(expected_count(from, to, 120), 120);
        assert_eq!(expected_count(from, to, 3600), 4);
        let empty = coverage_for("radar", "p", 120, from, to, &[]);
        assert_eq!(empty.missing_expected, 120);
        assert_eq!(empty.gaps.len(), 1);
        let objects = vec![
            (from + Duration::seconds(135), "a".into()),
            (to - Duration::seconds(225), "b".into()),
        ];
        let c = coverage_for("radar", "p", 120, from, to, &objects);
        assert_eq!(c.gaps.len(), 3);
        assert_eq!(c.keys, vec!["a", "b"]);
    }
}
