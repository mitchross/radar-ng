//! `ingest-star-seeing`: ECCC RDPS seeing and sky transparency for the stargazing score.
//!
//! Each hourly run picks the newest RDPS run that should be out, builds the file names from the
//! run time (never a directory listing), and fetches the night hours of the next
//! [`AHEAD_H`] hours that are missing or from an older run. Both fields are decoded with our
//! pure-Rust rotated-grid decoder and packed into one `STATE_DIR/star-seeing/<hour>.bin` per hour
//! ([`radar_grib::seeing`]). The first 404 or error ends the run; the last good files stay.
//!
//! ECCC's usage policy asks systematic Datamart users to use AMQPS rather than HTTPS, so the fetch
//! is off unless `ECCC_SEEING_ENABLED=true`. Without the files the star map scores on cloud,
//! darkness and moon alone.

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeDelta, Timelike, Utc};
use radar_grib::rotated::decode_rotated;
use radar_grib::seeing::{self, SEEING_DIR, SeeingHour};
use radar_types::payloads::sky::SkyResult;

use super::{PRODUCT_STAR_SEEING, SkyError};

/// Datamart root. Files live under the run date, so a run from yesterday still resolves.
pub const DATAMART_URL: &str = "https://dd.weather.gc.ca";
/// Credit line both clients show.
pub const CREDIT: &str = "Data Source: Environment and Climate Change Canada";
/// RDPS runs every 6 h.
pub const RUN_EVERY_H: i64 = 6;
/// A run is assumed complete this long after its start.
pub const RUN_READY_AFTER_H: i64 = 4;
/// Hours ahead fetched (the HRRR star map reaches 48 h; the card reads 18 h).
pub const AHEAD_H: i64 = 36;
/// Longest RDPS lead with astronomy fields.
pub const MAX_LEAD_H: i64 = 84;
/// Hours fetched per run at most, so one activity stays well inside its 60 s.
pub const MAX_HOURS_PER_RUN: usize = 12;
/// No new request starts after this much of the run.
pub const BUDGET: Duration = Duration::from_secs(40);
/// Files for hours this far in the past are removed.
pub const KEEP_PAST_H: i64 = 2;
/// Identifies us to ECCC.
pub const USER_AGENT: &str = concat!(
    "radar-ng/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/mitchross/radar-ng; star forecast, RDPS seeing and transparency)"
);

/// The two fields, as ECCC names them.
pub const FIELDS: [&str; 2] = ["SeeingIndex", "SkyTransparencyIndex"];

/// A client with our User-Agent that never follows redirects.
pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()
}

/// The newest RDPS run that should be complete at `now`.
#[must_use]
pub fn latest_run(now: DateTime<Utc>) -> DateTime<Utc> {
    let ready = now - TimeDelta::hours(RUN_READY_AFTER_H);
    let step = RUN_EVERY_H * 3600;
    DateTime::from_timestamp(ready.timestamp().div_euclid(step) * step, 0).unwrap_or(ready)
}

/// UTC hours that are night somewhere over North America; daytime hours are never fetched.
#[must_use]
pub fn is_night_hour(valid: DateTime<Utc>) -> bool {
    let h = valid.hour();
    h >= 23 || h <= 13
}

/// Valid hours to hold for `run` at `now`: the night hours from this hour to [`AHEAD_H`] ahead
/// that the run covers.
#[must_use]
pub fn wanted_hours(run: DateTime<Utc>, now: DateTime<Utc>) -> Vec<DateTime<Utc>> {
    let start = DateTime::from_timestamp(now.timestamp().div_euclid(3600) * 3600, 0).unwrap_or(now);
    (0..=AHEAD_H)
        .map(|h| start + TimeDelta::hours(h))
        .filter(|v| {
            let lead = (*v - run).num_hours();
            (1..=MAX_LEAD_H).contains(&lead) && is_night_hour(*v)
        })
        .collect()
}

/// `{root}/{YYYYMMDD}/WXO-DD/model_rdps/10km/{HH}/{hhh}/{run}_MSC_RDPS_{field}_EAtm_RLatLon0.09_PT{hhh}H.grib2`.
#[must_use]
pub fn file_url(root: &str, run: DateTime<Utc>, valid: DateTime<Utc>, field: &str) -> String {
    let lead = (valid - run).num_hours();
    format!(
        "{root}/{date}/WXO-DD/model_rdps/10km/{hh}/{lead:03}/{date}T{hh}Z_MSC_RDPS_{field}_EAtm_RLatLon0.09_PT{lead:03}H.grib2",
        date = run.format("%Y%m%d"),
        hh = run.format("%H"),
    )
}

/// The run stored for a valid hour, if its file is readable.
fn stored_run(dir: &Path, valid: DateTime<Utc>) -> Option<i64> {
    use std::io::Read;
    let mut head = [0u8; seeing::HEADER_LEN];
    fs::File::open(dir.join(seeing::file_name(valid)))
        .and_then(|mut f| f.read_exact(&mut head))
        .ok()?;
    seeing::parse_header(&head).map(|(_, run)| run)
}

/// Removes hour files older than [`KEEP_PAST_H`] and stray temp files.
fn prune(dir: &Path, now: DateTime<Utc>) -> std::io::Result<()> {
    let cutoff = seeing::file_name(now - TimeDelta::hours(KEEP_PAST_H));
    let Ok(entries) = fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Hour names sort in time order.
        let old = name.len() == cutoff.len() && name.ends_with(".bin") && *name < *cutoff;
        let stray_tmp = name.starts_with(".sky.") && name.ends_with(".tmp");
        if old || stray_tmp {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn result(outcome: &str, http_status: Option<i64>, run: Option<DateTime<Utc>>) -> SkyResult {
    SkyResult {
        product: PRODUCT_STAR_SEEING.to_owned(),
        outcome: outcome.to_owned(),
        http_status,
        epoch: run.map(|r| r.to_rfc3339()),
    }
}

enum Fetch {
    Ok(Vec<u8>),
    Stop(&'static str, Option<i64>),
}

async fn fetch(http: &reqwest::Client, url: &str) -> Fetch {
    let response = match http.get(url).send().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "eccc request failed; keeping the last files");
            return Fetch::Stop("transport-error", None);
        }
    };
    let status = response.status().as_u16();
    match status {
        200 => match response.bytes().await {
            Ok(b) => Fetch::Ok(b.to_vec()),
            Err(_) => Fetch::Stop("transport-error", Some(200)),
        },
        // Not out yet: the next hourly run asks again.
        404 => Fetch::Stop("pending", Some(404)),
        _ => {
            tracing::warn!(status, "eccc returned non-200; keeping the last files");
            Fetch::Stop("http-error", Some(i64::from(status)))
        }
    }
}

/// One `ingest-star-seeing` run against `root` (the Datamart, or a test server).
pub async fn ingest(
    http: &reqwest::Client,
    root: &str,
    state_dir: &Path,
    now: DateTime<Utc>,
    enabled: bool,
) -> Result<SkyResult, SkyError> {
    if !enabled {
        return Ok(result("disabled", None, None));
    }
    let dir = state_dir.join(SEEING_DIR);
    fs::create_dir_all(&dir)?;
    prune(&dir, now)?;
    let run = latest_run(now);
    let todo: Vec<DateTime<Utc>> = wanted_hours(run, now)
        .into_iter()
        .filter(|v| stored_run(&dir, *v).is_none_or(|r| r < run.timestamp()))
        .take(MAX_HOURS_PER_RUN)
        .collect();
    if todo.is_empty() {
        return Ok(result("up-to-date", None, Some(run)));
    }
    let started = Instant::now();
    for (written, valid) in todo.into_iter().enumerate() {
        if started.elapsed() > BUDGET {
            break;
        }
        let mut bytes = Vec::with_capacity(2);
        for field in FIELDS {
            match fetch(http, &file_url(root, run, valid, field)).await {
                Fetch::Ok(b) => bytes.push(b),
                Fetch::Stop(outcome, status) => {
                    let outcome = if written > 0 { "partial" } else { outcome };
                    return Ok(result(outcome, status, Some(run)));
                }
            }
        }
        let run_unix = run.timestamp();
        let packed = tokio::task::spawn_blocking(move || {
            let s = decode_rotated(&bytes[0]).ok()?;
            let t = decode_rotated(&bytes[1]).ok()?;
            SeeingHour::from_fields(&s, &t, run_unix).map(|h| h.encode())
        })
        .await
        .ok()
        .flatten();
        let Some(packed) = packed else {
            tracing::warn!(%valid, "eccc files did not decode; keeping the last files");
            return Ok(result("invalid", Some(200), Some(run)));
        };
        super::iss::write_public_atomic(&dir, &dir.join(seeing::file_name(valid)), &packed)?;
    }
    Ok(result("published", Some(200), Some(run)))
}

/// `(seeing, transparency)` at a point for a valid hour, reading one byte of the stored file.
#[must_use]
pub fn read_point(state_dir: &Path, valid: DateTime<Utc>, lat: f64, lon: f64) -> Option<(u8, u8)> {
    use std::os::unix::fs::FileExt;
    let f = fs::File::open(state_dir.join(SEEING_DIR).join(seeing::file_name(valid))).ok()?;
    let mut head = [0u8; seeing::HEADER_LEN];
    f.read_exact_at(&mut head, 0).ok()?;
    let (grid, _) = seeing::parse_header(&head)?;
    let offset = seeing::cell_offset(&grid, lat, lon)?;
    let mut cell = [0u8; 1];
    f.read_exact_at(&mut cell, offset).ok()?;
    seeing::unpack(cell[0])
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::TimeZone;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    const SEEING: &[u8] =
        include_bytes!("../../../radar-grib/testdata/eccc_rdps_seeing_f006_greatlakes.grib2");
    const TRANSPARENCY: &[u8] =
        include_bytes!("../../../radar-grib/testdata/eccc_rdps_transparency_f006_greatlakes.grib2");

    /// Serves the fixtures by field name, or `status` for every request. Records request paths.
    async fn server(status: u16) -> (String, Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let paths = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (counter, seen) = (Arc::clone(&hits), Arc::clone(&paths));
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_owned();
                seen.lock().unwrap().push(path.clone());
                let body: &[u8] = if status != 200 {
                    b""
                } else if path.contains("SkyTransparencyIndex") {
                    TRANSPARENCY
                } else {
                    SEEING
                };
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body).await;
            }
        });
        (url, hits, paths)
    }

    fn utc(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, d, h, m, 0).unwrap()
    }

    #[test]
    fn names_come_from_the_run_time() {
        assert_eq!(latest_run(utc(18, 6, 45)), utc(18, 0, 0));
        assert_eq!(latest_run(utc(18, 3, 59)), utc(17, 18, 0));
        assert_eq!(
            file_url(DATAMART_URL, utc(18, 0, 0), utc(18, 6, 0), "SeeingIndex"),
            "https://dd.weather.gc.ca/20261118/WXO-DD/model_rdps/10km/00/006/20261118T00Z_MSC_RDPS_SeeingIndex_EAtm_RLatLon0.09_PT006H.grib2"
        );
        assert!(USER_AGENT.contains("github.com/mitchross/radar-ng"));
    }

    #[test]
    fn only_night_hours_in_range_are_wanted() {
        let run = utc(18, 0, 0);
        let hours = wanted_hours(run, utc(18, 6, 45));
        assert_eq!(hours.first(), Some(&utc(18, 6, 0)));
        assert!(hours.iter().all(|v| is_night_hour(*v)));
        assert!(!hours.contains(&utc(18, 14, 0)));
        assert!(hours.contains(&utc(18, 23, 0)));
        assert!(hours.iter().all(|v| (*v - run).num_hours() <= MAX_LEAD_H));
        // Lead 0 has no astronomy fields.
        assert!(!wanted_hours(run, utc(18, 0, 10)).contains(&run));
    }

    #[tokio::test]
    async fn disabled_sends_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, _) = server(200).await;
        let r = ingest(&client().unwrap(), &url, dir.path(), utc(18, 6, 45), false)
            .await
            .unwrap();
        assert_eq!(r.outcome, "disabled");
        assert_eq!(hits.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn publishes_hours_then_is_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits, paths) = server(200).await;
        let http = client().unwrap();
        let now = utc(18, 6, 45);
        let r = ingest(&http, &url, dir.path(), now, true).await.unwrap();
        assert_eq!(r.outcome, "published");
        assert_eq!(r.product, "star-seeing");
        assert_eq!(hits.load(Ordering::SeqCst), 2 * MAX_HOURS_PER_RUN);
        assert!(paths.lock().unwrap()[0].starts_with("/20261118/WXO-DD/model_rdps/10km/00/006/"));
        assert_eq!(
            read_point(dir.path(), utc(18, 6, 0), 43.65, -79.38),
            Some((3, 3))
        );
        assert_eq!(read_point(dir.path(), utc(18, 6, 0), 25.76, -80.19), None);
        assert_eq!(read_point(dir.path(), utc(18, 15, 0), 43.65, -79.38), None);
        // Keep going until every wanted hour is held, then nothing more is asked.
        let wanted = wanted_hours(latest_run(now), now).len();
        for _ in 0..10 {
            if ingest(&http, &url, dir.path(), now, true)
                .await
                .unwrap()
                .outcome
                == "up-to-date"
            {
                break;
            }
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2 * wanted);
        let held = fs::read_dir(dir.path().join(SEEING_DIR)).unwrap().count();
        assert_eq!(held, wanted);
    }

    #[tokio::test]
    async fn a_404_stops_the_run_and_keeps_old_files() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _, _) = server(200).await;
        let http = client().unwrap();
        ingest(&http, &url, dir.path(), utc(18, 6, 45), true)
            .await
            .unwrap();
        // Six hours on, the 06Z run is not out yet.
        let (missing, hits, _) = server(404).await;
        let r = ingest(&http, &missing, dir.path(), utc(18, 10, 5), true)
            .await
            .unwrap();
        assert_eq!((r.outcome.as_str(), r.http_status), ("pending", Some(404)));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            read_point(dir.path(), utc(18, 10, 0), 43.65, -79.38),
            Some((3, 3))
        );
        // Old hours are pruned.
        assert!(
            !dir.path()
                .join(SEEING_DIR)
                .join("20261118T06Z.bin")
                .exists()
        );
    }

    #[tokio::test]
    async fn garbage_is_not_stored() {
        let dir = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 4\r\nconnection: close\r\n\r\nGRIB",
                    )
                    .await;
            }
        });
        let r = ingest(&client().unwrap(), &url, dir.path(), utc(18, 6, 45), true)
            .await
            .unwrap();
        assert_eq!(r.outcome, "invalid");
        assert_eq!(
            fs::read_dir(dir.path().join(SEEING_DIR)).unwrap().count(),
            0
        );
    }
}
