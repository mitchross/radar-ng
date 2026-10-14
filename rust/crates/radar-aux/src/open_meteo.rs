//! Open-Meteo sync: `open_meteo_sync` (spec-ingest-aux section 9).
//!
//! Runs `OPENMETEO_BIN sync <model> <variables> --past-days <n>` as a child process in the parent of
//! `OPENMETEO_DATA_DIR`, with the worker's environment. The child's stdout and stderr are drained line by
//! line, each keeping its last 50 lines. A heartbeat `{model, elapsed_s}` is sent every `heartbeat_every`.
//!
//! Success is decided from the output, not the exit code. The binary logs `[ CRITICAL ]` or `[ ERROR ]` and
//! still exits 0 (spec 9.3). Any such line in the kept tails fails the activity, and so does a non-zero exit.
//!
//! Cancellation kills the child (DECIDE D-9). The command uses `kill_on_drop`, so dropping the future
//! (Temporal cancel or timeout) sends SIGKILL.

use std::collections::VecDeque;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use radar_core::time::round_digits;
use radar_types::payloads::open_meteo::{OpenMeteoSyncArgs, OpenMeteoSyncResult};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::time::{MissedTickBehavior, interval_at};
use tracing::{debug, info};

use crate::beat::Beat;

/// Lines kept from each stream (`deque(maxlen=50)`, Q-12).
pub const TAIL_LINES: usize = 50;
/// Characters kept from an error message (`[-500:]`, Q-13).
pub const MESSAGE_TAIL_CHARS: usize = 500;

/// Configuration for the sync.
#[derive(Debug, Clone)]
pub struct OpenMeteoConfig {
    /// `OPENMETEO_BIN` (default `/app/openmeteo-api`).
    pub bin: PathBuf,
    /// `OPENMETEO_DATA_DIR` (default `/app/data`). Only its parent is used, as the working directory.
    pub data_dir: PathBuf,
    /// Heartbeat period (30 s).
    pub heartbeat_every: Duration,
}

impl OpenMeteoConfig {
    /// Reads `OPENMETEO_BIN` and `OPENMETEO_DATA_DIR` with the code defaults.
    pub fn from_env() -> Self {
        Self {
            bin: radar_core::env::path_var("OPENMETEO_BIN", "/app/openmeteo-api"),
            data_dir: radar_core::env::path_var("OPENMETEO_DATA_DIR", "/app/data"),
            heartbeat_every: Duration::from_secs(30),
        }
    }

    /// The child's working directory: the parent of the data directory (`/app`).
    pub fn working_dir(&self) -> PathBuf {
        match self.data_dir.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }
}

/// Failure of one sync. `retryable` mirrors Python's `ApplicationError(non_retryable=False)`: the workflow's
/// retry policy applies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct OpenMeteoError {
    /// Message, in the Python format (spec 9.3).
    pub message: String,
    /// Whether the workflow may retry.
    pub retryable: bool,
}

/// The exact argv (`[bin, "sync", model, variables, "--past-days", n]`).
pub fn command_argv(cfg: &OpenMeteoConfig, args: &OpenMeteoSyncArgs) -> Vec<String> {
    vec![
        cfg.bin.to_string_lossy().into_owned(),
        "sync".to_string(),
        args.model.clone(),
        args.variables.clone(),
        "--past-days".to_string(),
        args.past_days.to_string(),
    ]
}

/// The first line containing `[ CRITICAL ]` or `[ ERROR ]`, as it was printed (Python `critical_error`).
pub fn critical_error<'a, I>(lines: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    lines
        .into_iter()
        .find(|line| line.contains("[ CRITICAL ]") || line.contains("[ ERROR ]"))
}

/// The last `n` characters of `s` (Python `s[-n:]`, counted in code points).
pub fn last_chars(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        s.to_string()
    } else {
        s.chars().skip(count - n).collect()
    }
}

/// Runs the sync and classifies the result. `beat` receives `{model, elapsed_s}` every `heartbeat_every`.
pub async fn open_meteo_sync(
    cfg: &OpenMeteoConfig,
    args: &OpenMeteoSyncArgs,
    beat: Beat,
) -> Result<OpenMeteoSyncResult, OpenMeteoError> {
    let started = Instant::now();
    let argv = command_argv(cfg, args);
    info!(command = %argv.join(" "), "starting open-meteo sync");

    // ETXTBSY ("Text file busy") is transient: another thread forked while a write handle to the
    // binary was still open (a binary just written by an installer or a test). Retry briefly.
    let mut attempt = 0;
    let mut child = loop {
        let spawned = Command::new(&argv[0])
            .args(&argv[1..])
            .current_dir(cfg.working_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        match spawned {
            Err(e) if e.raw_os_error() == Some(26) && attempt < 20 => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            other => {
                break other.map_err(|e| OpenMeteoError {
                    message: format!("failed to start {}: {e}", argv[0]),
                    retryable: true,
                })?;
            }
        }
    };

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_task = tokio::spawn(drain(stdout));
    let err_task = tokio::spawn(drain(stderr));

    let mut ticker = interval_at(
        tokio::time::Instant::now() + cfg.heartbeat_every,
        cfg.heartbeat_every,
    );
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let status = loop {
        tokio::select! {
            status = child.wait() => {
                break status.map_err(|e| OpenMeteoError { message: format!("waiting for open-meteo failed: {e}"), retryable: true })?;
            }
            _ = ticker.tick() => {
                beat(json!({ "model": args.model, "elapsed_s": started.elapsed().as_secs() }));
            }
        }
    };
    let stdout_tail = out_task.await.unwrap_or_default();
    let stderr_tail = err_task.await.unwrap_or_default();
    let duration_s = round_digits(started.elapsed().as_secs_f64(), 1);
    let rc = return_code(&status);

    let critical = critical_error(
        stdout_tail
            .iter()
            .chain(stderr_tail.iter())
            .map(String::as_str),
    );
    if let Some(line) = critical {
        return Err(OpenMeteoError {
            message: format!(
                "open-meteo {} sync reported an error (rc={rc}): {}",
                args.model,
                last_chars(line, MESSAGE_TAIL_CHARS)
            ),
            retryable: true,
        });
    }
    if rc != 0 {
        let tail = stderr_tail.iter().cloned().collect::<Vec<_>>().join("\n");
        return Err(OpenMeteoError {
            message: format!(
                "open-meteo {} sync exited rc={rc}: {}",
                args.model,
                last_chars(&tail, MESSAGE_TAIL_CHARS)
            ),
            retryable: true,
        });
    }
    info!(model = %args.model, duration_s, "open-meteo sync ok");
    Ok(OpenMeteoSyncResult {
        model: args.model.clone(),
        succeeded: true,
        duration_s,
        return_code: rc as i64,
        stderr_tail: String::new(),
    })
}

/// Python `returncode or 0`, with a signal reported as the negative signal number.
fn return_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    status.signal().map(|s| -s).unwrap_or(-1)
}

/// Reads a stream to EOF, keeping the last `TAIL_LINES` lines with trailing whitespace removed.
async fn drain<R: AsyncRead + Unpin>(stream: R) -> VecDeque<String> {
    let mut tail: VecDeque<String> = VecDeque::with_capacity(TAIL_LINES);
    let mut reader = BufReader::new(stream);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).trim_end().to_string();
                debug!(line = %line, "open-meteo output");
                if tail.len() == TAIL_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    }
    tail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_error_flags_the_linkat_line_verbatim() {
        let lines = [
            "[ INFO ] Downloading 2 files (147.4 MB)",
            "[ CRITICAL ] Error during sync linkAt(error: -1)",
        ];
        assert_eq!(
            critical_error(lines),
            Some("[ CRITICAL ] Error during sync linkAt(error: -1)")
        );
    }

    #[test]
    fn error_lines_count_and_clean_output_passes() {
        assert!(critical_error(["[ ERROR ] Could not download chunk"]).is_some());
        assert!(critical_error(["[ INFO ] ncep_hrrr_conus 16 files completed"]).is_none());
    }

    #[test]
    fn last_chars_counts_code_points() {
        assert_eq!(last_chars("abcdef", 3), "def");
        assert_eq!(last_chars("ab", 3), "ab");
        assert_eq!(last_chars("ééé", 2), "éé");
    }

    #[test]
    fn argv_matches_the_python_command() {
        let cfg = OpenMeteoConfig {
            bin: "/app/openmeteo-api".into(),
            data_dir: "/app/data".into(),
            heartbeat_every: Duration::from_secs(30),
        };
        let args = OpenMeteoSyncArgs {
            model: "ncep_gfs013".into(),
            variables: "temperature_2m,cloud_cover".into(),
            past_days: 1,
        };
        assert_eq!(
            command_argv(&cfg, &args),
            vec![
                "/app/openmeteo-api",
                "sync",
                "ncep_gfs013",
                "temperature_2m,cloud_cover",
                "--past-days",
                "1"
            ]
        );
        assert_eq!(cfg.working_dir(), PathBuf::from("/app"));
    }
}
