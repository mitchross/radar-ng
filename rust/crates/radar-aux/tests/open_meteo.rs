//! `open_meteo_sync` against fake binaries (shell scripts) in a tempdir (spec-ingest-aux section 9).

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::recording_beat;
use radar_aux::open_meteo::{OpenMeteoConfig, open_meteo_sync};
use radar_types::payloads::open_meteo::OpenMeteoSyncArgs;
use tempfile::TempDir;

struct Fake {
    dir: TempDir,
    bin: PathBuf,
    cfg: OpenMeteoConfig,
}

/// Writes a fake `openmeteo-api` with the given shell body. The body runs after the shell preamble.
fn fake(body: &str) -> Fake {
    let dir = TempDir::new().unwrap();
    let app = dir.path().join("app");
    std::fs::create_dir_all(app.join("data")).unwrap();
    let bin = dir.path().join("openmeteo-api");
    std::fs::write(
        &bin,
        format!("#!/bin/sh\nOUT='{}'\n{body}\n", dir.path().display()),
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let cfg = OpenMeteoConfig {
        bin: bin.clone(),
        data_dir: app.join("data"),
        heartbeat_every: Duration::from_secs(30),
    };
    Fake { dir, bin, cfg }
}

fn args(model: &str) -> OpenMeteoSyncArgs {
    OpenMeteoSyncArgs {
        model: model.to_string(),
        variables: "temperature_2m,cloud_cover".to_string(),
        past_days: 1,
    }
}

#[tokio::test]
async fn success_passes_argv_and_working_directory_and_reports_the_result() {
    let f = fake(
        "printf '%s\\n' \"$@\" > \"$OUT/argv.txt\"\npwd > \"$OUT/cwd.txt\"\necho '[ INFO ] ncep_gfs013 completed'\n",
    );
    let (beat, _) = recording_beat();
    let res = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap();

    assert!(res.succeeded);
    assert_eq!(res.model, "ncep_gfs013");
    assert_eq!(res.return_code, 0);
    assert_eq!(res.stderr_tail, "", "success returns an empty stderr_tail");
    assert!(res.duration_s >= 0.0);

    let argv = std::fs::read_to_string(f.dir.path().join("argv.txt")).unwrap();
    assert_eq!(
        argv.lines().collect::<Vec<_>>(),
        vec![
            "sync",
            "ncep_gfs013",
            "temperature_2m,cloud_cover",
            "--past-days",
            "1"
        ]
    );
    let cwd = std::fs::read_to_string(f.dir.path().join("cwd.txt")).unwrap();
    assert_eq!(
        Path::new(cwd.trim()).canonicalize().unwrap(),
        f.dir.path().join("app").canonicalize().unwrap()
    );
}

#[tokio::test]
async fn critical_line_fails_even_with_exit_code_zero() {
    let f = fake("echo '[ CRITICAL ] Error during sync linkAt(error: -1)' >&2\nexit 0\n");
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap_err();
    assert!(err.retryable);
    assert_eq!(
        err.message,
        "open-meteo ncep_gfs013 sync reported an error (rc=0): [ CRITICAL ] Error during sync linkAt(error: -1)"
    );
}

#[tokio::test]
async fn error_line_on_stdout_fails_the_sync() {
    let f = fake("echo '[ ERROR ] Could not download chunk 7'\nexit 0\n");
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&f.cfg, &args("ncep_hrrr_conus"), beat)
        .await
        .unwrap_err();
    assert!(
        err.message
            .contains("reported an error (rc=0): [ ERROR ] Could not download chunk 7"),
        "{}",
        err.message
    );
}

#[tokio::test]
async fn non_zero_exit_reports_the_stderr_tail() {
    let f = fake("echo boom >&2\nexit 3\n");
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap_err();
    assert_eq!(err.message, "open-meteo ncep_gfs013 sync exited rc=3: boom");
}

#[tokio::test]
async fn message_keeps_only_the_last_500_characters() {
    let f =
        fake("head -c 2000 /dev/zero | tr '\\0' 'a' >&2\necho >&2\necho tail-marker >&2\nexit 1\n");
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap_err();
    let prefix = "open-meteo ncep_gfs013 sync exited rc=1: ";
    let body = err.message.strip_prefix(prefix).expect("prefix");
    assert_eq!(body.chars().count(), 500);
    assert!(body.ends_with("tail-marker"));
}

#[tokio::test]
async fn only_the_last_50_lines_of_each_stream_are_kept() {
    // 120 lines on stderr, exit 1: the message is built from lines 071..120 only.
    let f = fake(
        "i=1; while [ $i -le 120 ]; do printf 'line %03d\\n' $i >&2; i=$((i+1)); done\nexit 1\n",
    );
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap_err();
    assert!(err.message.contains("line 120"));
    assert!(err.message.contains("line 071"));
    assert!(!err.message.contains("line 070"), "older lines are dropped");
}

#[tokio::test]
async fn critical_line_outside_the_kept_window_does_not_fail() {
    // Python keeps the last 50 stdout lines: a CRITICAL printed earlier is not inspected.
    let f = fake(
        "echo '[ CRITICAL ] early'\ni=1; while [ $i -le 60 ]; do echo \"[ INFO ] step $i\"; i=$((i+1)); done\nexit 0\n",
    );
    let (beat, _) = recording_beat();
    let res = open_meteo_sync(&f.cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap();
    assert!(res.succeeded);
}

#[tokio::test]
async fn heartbeats_report_model_and_elapsed_time_while_running() {
    let f = fake("sleep 0.6\nexit 0\n");
    let mut cfg = f.cfg.clone();
    cfg.heartbeat_every = Duration::from_millis(100);
    let (beat, log) = recording_beat();
    open_meteo_sync(&cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap();
    let beats = log.lock().unwrap().clone();
    assert!(
        beats.len() >= 2,
        "expected several heartbeats, got {}",
        beats.len()
    );
    assert!(
        beats
            .iter()
            .all(|b| b["model"] == "ncep_gfs013" && b["elapsed_s"].is_u64())
    );
}

#[tokio::test]
async fn cancellation_kills_the_child_process() {
    let f = fake("echo $$ > \"$OUT/pid.txt\"\nexec sleep 30\n");
    let (beat, _) = recording_beat();
    let a = args("ncep_gfs013");
    let fut = open_meteo_sync(&f.cfg, &a, beat);
    // The caller gives up (as a Temporal cancel or timeout would).
    let timed = tokio::time::timeout(Duration::from_millis(800), fut).await;
    assert!(timed.is_err(), "the sync is still running after 800 ms");

    let pid: u32 = std::fs::read_to_string(f.dir.path().join("pid.txt"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    for _ in 0..50 {
        // A killed and not yet reaped child shows as a zombie (state Z), which is also dead.
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        if status.is_empty()
            || status
                .lines()
                .any(|l| l.starts_with("State:") && l.contains('Z'))
        {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(gone, "child process {pid} survived cancellation");
}

#[tokio::test]
async fn missing_binary_is_a_retryable_error() {
    let f = fake("exit 0\n");
    let mut cfg = f.cfg.clone();
    cfg.bin = f.dir.path().join("does-not-exist");
    let (beat, _) = recording_beat();
    let err = open_meteo_sync(&cfg, &args("ncep_gfs013"), beat)
        .await
        .unwrap_err();
    assert!(err.retryable);
    assert!(
        err.message.starts_with("failed to start"),
        "{}",
        err.message
    );
    let _ = f.bin;
}
