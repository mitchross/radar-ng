//! Nowcast skill invariants NSK-01..NSK-07 (spec-shared §13.5) and the golden replay of `testdata/skill/`.

use super::*;
use chrono::{Duration as ChronoDuration, TimeZone, Utc};
use serde_json::{Map, Value, json};
use std::fs;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap()
}

fn counts(hits: i64, misses: i64, false_alarms: i64) -> Value {
    json!({"hits": hits, "misses": misses, "false_alarms": false_alarms})
}

/// One scored run at one lead, with rain counts and persistence rain counts. Heavy counts are zero.
fn entry(
    anchor: &str,
    lead: i64,
    scored: DateTime<Utc>,
    rain: (i64, i64, i64),
    persistence: (i64, i64, i64),
) -> Map<String, Value> {
    json!({
        "anchor": anchor,
        "valid": "v",
        "lead_minutes": lead,
        "scored_at": scored.to_rfc3339(),
        "cells": 100,
        "thresholds": {"rain": counts(rain.0, rain.1, rain.2), "heavy": counts(0, 0, 0)},
        "persistence": {"rain": counts(persistence.0, persistence.1, persistence.2), "heavy": counts(0, 0, 0)},
    })
    .as_object()
    .unwrap()
    .clone()
}

fn log_of(entries: Vec<Map<String, Value>>) -> SkillLog {
    SkillLog {
        updated_at: Some("2026-09-30T12:00:00+00:00".to_string()),
        window_hours: Some(24),
        entries: entries.into_iter().map(Value::Object).collect(),
        extra: Map::new(),
    }
}

/// Six anchors at lead 30 (rain 10/2/3, persistence 6/6/6) and at lead 5 (rain 20/0/0, persistence 6/6/6).
fn six_run_log() -> SkillLog {
    let mut entries = Vec::new();
    for i in 0..6 {
        let scored = now() - ChronoDuration::minutes(i + 1);
        entries.push(entry(&format!("a{i}"), 30, scored, (10, 2, 3), (6, 6, 6)));
        entries.push(entry(&format!("a{i}"), 5, scored, (20, 0, 0), (6, 6, 6)));
    }
    log_of(entries)
}

#[test]
fn nsk_01_scores_are_the_classic_contingency_table() {
    let s = scores_from_counts(&Counts {
        hits: 8,
        misses: 2,
        false_alarms: 4,
    });
    assert_eq!(s.pod, Some(0.8));
    assert_eq!(s.far, Some(0.333));
    assert_eq!(s.csi, Some(0.571));
    assert_eq!(s.bias, Some(1.2));

    let empty = scores_from_counts(&Counts::default());
    assert_eq!(
        (empty.pod, empty.far, empty.csi, empty.bias),
        (None, None, None, None)
    );

    let no_observed_rain = scores_from_counts(&Counts {
        hits: 0,
        misses: 0,
        false_alarms: 5,
    });
    assert_eq!(no_observed_rain.pod, None);
    assert_eq!(no_observed_rain.far, Some(1.0));
    assert_eq!(no_observed_rain.csi, Some(0.0));
    assert_eq!(no_observed_rain.bias, None);
}

#[test]
fn nsk_02_append_keeps_the_window_sorts_and_leaves_exactly_one_file() {
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state");
    let cfg = SkillConfig::default();
    let n = now();
    let entries = vec![
        entry(
            "old",
            30,
            n - ChronoDuration::hours(30),
            (1, 0, 0),
            (0, 0, 0),
        ),
        entry(
            "b",
            30,
            n - ChronoDuration::minutes(1),
            (1, 0, 0),
            (0, 0, 0),
        ),
        entry(
            "a",
            30,
            n - ChronoDuration::minutes(2),
            (1, 0, 0),
            (0, 0, 0),
        ),
        entry("a", 5, n - ChronoDuration::minutes(2), (1, 0, 0), (0, 0, 0)),
        entry(
            "future",
            15,
            n + ChronoDuration::minutes(2),
            (1, 0, 0),
            (0, 0, 0),
        ),
    ];
    let log = append_entries(&state, &entries, n, None, &cfg).unwrap();

    let mut names: Vec<String> = fs::read_dir(&state)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // The lock sidecar is the one deliberate extra file (F4); no temp file is left behind.
    assert_eq!(
        names,
        vec![SKILL_FILE.to_string(), LOCK_FILE.to_string()],
        "no temp file left behind"
    );

    let order: Vec<(String, i64)> = log
        .entries
        .iter()
        .map(|e| {
            (
                e["anchor"].as_str().unwrap().to_string(),
                e["lead_minutes"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        order,
        vec![
            ("a".into(), 5),
            ("a".into(), 30),
            ("b".into(), 30),
            ("future".into(), 15)
        ],
        "old entry dropped; ties broken by lead; future kept"
    );
    assert_eq!(log.updated_at.as_deref(), Some("2026-09-30T12:00:00+00:00"));

    // Every append re-applies the window, even when nothing new is added (DRV-10).
    let later = n + ChronoDuration::hours(24) + ChronoDuration::minutes(30);
    let after = append_entries(&state, &[], later, None, &cfg).unwrap();
    assert!(after.entries.is_empty());
}

#[test]
fn nsk_03_unreadable_or_wrong_shape_logs_read_as_empty() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = SkillConfig::default();
    assert!(
        read_log(dir.path(), &cfg).entries.is_empty(),
        "missing file"
    );
    for body in ["not json", "[1, 2, 3]", r#"{"updated_at": "x"}"#] {
        fs::write(skill_path(dir.path()), body).unwrap();
        let log = read_log(dir.path(), &cfg);
        assert!(log.entries.is_empty(), "{body}");
        assert_eq!(log.updated_at, None, "{body}");
    }
}

#[test]
fn nsk_03_append_refuses_to_overwrite_an_unreadable_log() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = SkillConfig::default();
    fs::write(skill_path(dir.path()), "not json").unwrap();
    let err = append_entries(dir.path(), &[], now(), None, &cfg).unwrap_err();
    assert!(matches!(err, SkillError::CorruptLog { .. }), "{err:?}");
    assert_eq!(
        fs::read_to_string(skill_path(dir.path())).unwrap(),
        "not json"
    );
}

#[test]
fn nsk_04_headline_is_withheld_until_enough_runs_are_scored() {
    let cfg = SkillConfig::default();
    let five: Vec<_> = (0..5)
        .map(|i| {
            entry(
                &format!("a{i}"),
                30,
                now() - ChronoDuration::minutes(1),
                (10, 2, 3),
                (6, 6, 6),
            )
        })
        .collect();
    let summary = summarize(&log_of(five), now(), None, &cfg);
    assert!(!summary.available);
    assert_eq!(summary.reason, Some("warming_up"));
    assert!(summary.headline.is_none());
    assert_eq!(summary.runs_scored, 5);

    let empty = summarize(&log_of(vec![]), now(), None, &cfg);
    assert_eq!(empty.reason, Some("no_scored_runs"));
    assert!(empty.by_lead.is_empty());
}

#[test]
fn summary_is_stable_until_the_oldest_counted_entry_leaves_the_window() {
    let cfg = SkillConfig::default();
    let log = six_run_log();
    // The oldest entry was scored 6 minutes ago; the window is 24 h.
    let expiry = summary_expiry(&log, now(), None, &cfg).expect("entries are counted");
    assert_eq!(
        expiry,
        now() - ChronoDuration::minutes(6) + ChronoDuration::hours(24)
    );
    let before = summarize(&log, expiry - ChronoDuration::seconds(1), None, &cfg);
    assert_eq!(before, summarize(&log, now(), None, &cfg));
    assert_ne!(
        summarize(&log, expiry + ChronoDuration::seconds(1), None, &cfg),
        before
    );
    assert_eq!(summary_expiry(&log_of(vec![]), now(), None, &cfg), None);
}

#[test]
fn nsk_05_per_lead_scores_and_the_30_minute_headline() {
    let cfg = SkillConfig::default();
    let summary = summarize(&six_run_log(), now(), None, &cfg);
    assert!(summary.available, "{:?}", summary.reason);
    assert_eq!(summary.runs_scored, 6);
    assert_eq!(
        summary
            .by_lead
            .iter()
            .map(|r| r.lead_minutes)
            .collect::<Vec<_>>(),
        vec![5, 30]
    );

    let lead30 = &summary.by_lead[1];
    assert_eq!(lead30.runs, 6);
    assert_eq!(
        (
            lead30.rain.scores.hits,
            lead30.rain.scores.misses,
            lead30.rain.scores.false_alarms
        ),
        (60, 12, 18)
    );
    assert_eq!(lead30.rain.scores.pod, Some(0.833), "60 / 72");
    assert_eq!(lead30.rain.scores.far, Some(0.231), "18 / 78");
    assert_eq!(lead30.rain.scores.csi, Some(0.667), "60 / 90");
    assert_eq!(lead30.rain.persistence_csi, Some(0.333), "36 / 108");

    let lead5 = &summary.by_lead[0];
    assert_eq!(lead5.rain.scores.pod, Some(1.0));
    assert_eq!(lead5.rain.scores.far, Some(0.0));
    assert_eq!(lead5.rain.scores.csi, Some(1.0));

    let headline = summary.headline.expect("headline");
    assert_eq!(headline.lead_minutes, 30);
    assert_eq!(headline.window_hours, 24);
    assert_eq!(headline.runs, 6);
    assert_eq!(headline.pod, Some(0.833));
    assert_eq!(headline.far, Some(0.231));
    assert_eq!(headline.csi, Some(0.667));
    assert_eq!(headline.persistence_csi, Some(0.333));
}

#[test]
fn nsk_06_no_rain_observed_withholds_the_headline() {
    let cfg = SkillConfig::default();
    let entries: Vec<_> = (0..6)
        .map(|i| {
            entry(
                &format!("a{i}"),
                30,
                now() - ChronoDuration::minutes(1),
                (0, 0, 4),
                (0, 0, 0),
            )
        })
        .collect();
    let summary = summarize(&log_of(entries), now(), None, &cfg);
    assert!(!summary.available);
    assert_eq!(summary.reason, Some("no_rain_observed"));
    assert!(summary.headline.is_none());
}

#[test]
fn nsk_07_prometheus_lines_expose_each_lead_and_threshold() {
    let cfg = SkillConfig::default();
    let summary = summarize(&six_run_log(), now(), None, &cfg);
    let lines = prometheus_lines(&summary);
    assert!(lines.contains(&"radar_ng_nowcast_skill_runs 6".to_string()));
    assert!(lines.contains(
        &r#"radar_ng_nowcast_pod{lead_minutes="30",threshold="rain"} 0.833"#.to_string()
    ));
    assert!(
        lines.contains(
            &r#"radar_ng_nowcast_csi{lead_minutes="5",threshold="rain"} 1.0"#.to_string()
        )
    );
    for metric in ["pod", "far", "csi"] {
        assert!(
            lines.contains(&format!("# TYPE radar_ng_nowcast_{metric} gauge")),
            "{metric}"
        );
    }

    let empty = prometheus_lines(&summarize(&log_of(vec![]), now(), None, &cfg));
    assert_eq!(
        empty,
        vec![
            "# TYPE radar_ng_nowcast_skill_runs gauge".to_string(),
            "radar_ng_nowcast_skill_runs 0".to_string()
        ]
    );
}

/// Golden replay of `testdata/skill/` produced by the Python module.
mod golden {
    use super::*;

    #[test]
    fn golden_skill_outputs_match_python() {
        let entries: Vec<Map<String, Value>> =
            serde_json::from_str(include_str!("../../testdata/skill/entries.json")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let cfg = SkillConfig::default();
        let log = append_entries(dir.path(), &entries, now(), None, &cfg).unwrap();

        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec![SKILL_FILE.to_string(), LOCK_FILE.to_string()]);
        let written = fs::read_to_string(skill_path(dir.path())).unwrap();
        assert_eq!(
            written,
            include_str!("../../testdata/skill/nowcast-skill.json")
        );

        let summary = summarize(&log, now(), None, &cfg);
        let got = serde_json::to_value(&summary).unwrap();
        let want: Value =
            serde_json::from_str(include_str!("../../testdata/skill/summary.json")).unwrap();
        assert_eq!(got, want);

        let prom = prometheus_lines(&summary).join("\n") + "\n";
        assert_eq!(prom, include_str!("../../testdata/skill/prometheus.txt"));

        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../../testdata/skill/ratios.json")).unwrap();
        for case in cases {
            let num = case["num"].as_i64().unwrap();
            let den = case["den"].as_i64().unwrap();
            let got = match ratio(num, den) {
                None => "None".to_string(),
                Some(v) => pycompat::float_repr(v),
            };
            assert_eq!(got, case["repr"].as_str().unwrap(), "ratio({num}, {den})");
        }
    }
}

/// F4 regression: appends take the `nowcast-skill.lock` sidecar exclusively. A lock held elsewhere makes an
/// append wait, and the append completes once it is released. The directory itself is not locked.
#[test]
fn appends_wait_on_the_read_write_lock_sidecar() {
    use std::sync::mpsc;
    use std::time::Duration as StdDuration;

    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().to_path_buf();
    let lock_path = state.join(LOCK_FILE);
    fs::write(&lock_path, b"").unwrap();
    let held = crate::fsutil::FlockGuard::exclusive(&lock_path).unwrap();

    let (tx, rx) = mpsc::channel();
    let worker_state = state.clone();
    std::thread::spawn(move || {
        let cfg = SkillConfig::default();
        let result = append_entries(&worker_state, &[], now(), None, &cfg).map(|_| ());
        let _ = tx.send(result.is_ok());
    });
    assert!(
        rx.recv_timeout(StdDuration::from_millis(300)).is_err(),
        "append must wait while the lock is held"
    );
    drop(held);
    let completed = rx
        .recv_timeout(StdDuration::from_secs(10))
        .expect("append completes after release");
    assert!(completed);
    assert!(lock_path.is_file(), "the sidecar is a regular file");
}
