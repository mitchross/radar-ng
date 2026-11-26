//! `ingest-iss`: the ISS orbital elements from CelesTrak.
//!
//! CelesTrak's usage policy (<https://celestrak.org/usage-policy.php>) asks for few requests, a clear
//! User-Agent and no hammering after errors. So each 6 h slot gets at most one request, the attempt is
//! recorded before it is sent, redirects are not followed, and any non-200 keeps the last good file.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, Utc};
use radar_types::payloads::sky::SkyResult;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{PRODUCT_ISS, SkyError};

/// The only URL this product fetches: the OMM elements of NORAD 25544, `.org` host, CSV.
pub const CELESTRAK_ISS_URL: &str =
    "https://celestrak.org/NORAD/elements/gp.php?CATNR=25544&FORMAT=CSV";
/// NORAD catalog number of the ISS.
pub const ISS_NORAD_ID: u64 = 25544;
/// One request per slot of this length.
pub const SLOT_S: i64 = 6 * 60 * 60;
/// The elements file the API reads.
pub const ISS_FILE: &str = "iss.json";
/// The last attempted slot, so a rerun in the same slot sends nothing.
pub const ATTEMPT_FILE: &str = "iss-attempt.json";
/// Identifies us to CelesTrak.
pub const USER_AGENT: &str = concat!(
    "radar-ng/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/mitchross/radar-ng; ISS pass predictions, one request per 6 h)"
);

/// `STATE_DIR/iss.json`: parsed elements and when we fetched them. Never re-served raw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssFile {
    /// When the elements were fetched.
    pub fetched_at: DateTime<Utc>,
    /// The OMM mean elements.
    pub elements: sgp4::Elements,
}

#[derive(Debug, Serialize, Deserialize)]
struct Attempt {
    slot: i64,
    at: DateTime<Utc>,
    outcome: String,
}

/// A client with our User-Agent that never follows redirects.
pub fn client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
}

/// The 6 h slot that contains `now`.
pub fn slot_of(now: DateTime<Utc>) -> i64 {
    now.timestamp().div_euclid(SLOT_S)
}

/// Parse a CelesTrak OMM CSV (header row plus one row per object) into the first object's elements.
pub fn parse_omm_csv(text: &str) -> Option<sgp4::Elements> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let header = split_csv(lines.next()?);
    let row = split_csv(lines.next()?);
    if header.len() != row.len() {
        return None;
    }
    let map: Map<String, Value> = header
        .into_iter()
        .zip(row)
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    serde_json::from_value(Value::Object(map)).ok()
}

fn split_csv(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut field)),
            _ => field.push(c),
        }
    }
    out.push(field);
    out
}

/// Read `STATE_DIR/iss.json`, `None` when missing or unreadable.
pub fn read_iss(state_dir: &Path) -> Option<IssFile> {
    let bytes = radar_core::state_pg::read_doc_or_disk(&state_dir.join(ISS_FILE), Some(ISS_FILE))
        .ok()
        .flatten()?;
    serde_json::from_slice(&bytes).ok()
}

fn last_attempt_slot(state_dir: &Path) -> Option<i64> {
    let bytes =
        radar_core::state_pg::read_doc_or_disk(&state_dir.join(ATTEMPT_FILE), Some(ATTEMPT_FILE))
            .ok()
            .flatten()?;
    serde_json::from_slice::<Attempt>(&bytes)
        .ok()
        .map(|a| a.slot)
}

fn record(state_dir: &Path, slot: i64, now: DateTime<Utc>, outcome: &str) -> std::io::Result<()> {
    let attempt = Attempt {
        slot,
        at: now,
        outcome: outcome.to_owned(),
    };
    let bytes = serde_json::to_vec(&attempt).map_err(std::io::Error::other)?;
    radar_core::state_pg::write_or_mirror(Some(ATTEMPT_FILE), &bytes, || {
        write_public_atomic(state_dir, &state_dir.join(ATTEMPT_FILE), &bytes)
    })
}

fn result(outcome: &str, http_status: Option<i64>, epoch: Option<String>) -> SkyResult {
    SkyResult {
        product: PRODUCT_ISS.to_owned(),
        outcome: outcome.to_owned(),
        http_status,
        epoch,
    }
}

/// One `ingest-iss` run: at most one request to `url`, and only if this slot has not been tried.
pub async fn ingest(
    http: &reqwest::Client,
    url: &str,
    state_dir: &Path,
    now: DateTime<Utc>,
) -> Result<SkyResult, SkyError> {
    if !radar_core::state_pg::read_enabled() {
        fs::create_dir_all(state_dir)?;
    }
    let slot = slot_of(now);
    if last_attempt_slot(state_dir) == Some(slot) {
        return Ok(result("skipped-slot", None, None));
    }
    // Recorded before the request, so a crash or rerun cannot send a second one in this slot.
    record(state_dir, slot, now, "started")?;
    let response = match http.get(url).send().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "celestrak request failed; keeping the last elements");
            record(state_dir, slot, now, "transport-error")?;
            return Ok(result("transport-error", None, None));
        }
    };
    let status = response.status().as_u16();
    if status != 200 {
        tracing::warn!(
            status,
            "celestrak returned non-200; keeping the last elements"
        );
        record(state_dir, slot, now, "http-error")?;
        return Ok(result("http-error", Some(i64::from(status)), None));
    }
    let body = response.text().await.unwrap_or_default();
    let Some(elements) = parse_omm_csv(&body).filter(|e| e.norad_id == ISS_NORAD_ID) else {
        tracing::warn!("celestrak body is not ISS OMM CSV; keeping the last elements");
        record(state_dir, slot, now, "invalid")?;
        return Ok(result("invalid", Some(200), None));
    };
    let epoch = elements.datetime.and_utc().to_rfc3339();
    let file = IssFile {
        fetched_at: now,
        elements,
    };
    let bytes = serde_json::to_vec(&file).map_err(std::io::Error::other)?;
    radar_core::state_pg::write_or_mirror(Some(ISS_FILE), &bytes, || {
        write_public_atomic(state_dir, &state_dir.join(ISS_FILE), &bytes)
    })?;
    record(state_dir, slot, now, "published")?;
    Ok(result("published", Some(200), Some(epoch)))
}

/// Temp file in `dir`, fsynced, 0644 so the API can read it, renamed over `path`, directory fsynced.
pub(super) fn write_public_atomic(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut tmp = tempfile::Builder::new()
        .prefix(".sky.")
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o644))?;
    tmp.persist(path).map_err(|e| e.error)?;
    radar_core::fsutil::fsync_dir(dir)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::TimeZone;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    const FIXTURE: &str = include_str!("../../testdata/sky/iss-25544-2026-11-17.csv");

    /// A one-route HTTP server that answers every request with `status` and `body`, counting requests.
    async fn server(status: u16, body: &'static str) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/gp.php", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        (url, hits)
    }

    fn at(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 17, h, 5, 0).unwrap()
    }

    #[test]
    fn url_is_the_org_host_over_https() {
        let url = url::Url::parse(CELESTRAK_ISS_URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("celestrak.org"));
        assert!(CELESTRAK_ISS_URL.contains("CATNR=25544"));
        assert!(USER_AGENT.contains("github.com/mitchross/radar-ng"));
    }

    #[test]
    fn parses_the_recorded_csv() {
        let e = parse_omm_csv(FIXTURE).expect("fixture parses");
        assert_eq!(e.norad_id, ISS_NORAD_ID);
        assert_eq!(e.object_name.as_deref(), Some("ISS (ZARYA)"));
        assert!((e.inclination - 51.6318).abs() < 1e-9);
        assert!((e.mean_motion - 15.494_353_3).abs() < 1e-9);
        assert_eq!(
            e.datetime.and_utc().to_rfc3339(),
            "2026-11-17T12:40:23.465568+00:00"
        );
        assert!(parse_omm_csv("No GP data found").is_none());
    }

    #[tokio::test]
    async fn non_200_is_tried_once_and_keeps_the_last_file() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits) = server(200, FIXTURE).await;
        let http = client().unwrap();
        let first = ingest(&http, &url, dir.path(), at(1)).await.unwrap();
        assert_eq!(first.outcome, "published");
        let good = fs::read(dir.path().join(ISS_FILE)).unwrap();

        let (bad_url, bad_hits) = server(503, "busy").await;
        let r = ingest(&http, &bad_url, dir.path(), at(7)).await.unwrap();
        assert_eq!(
            (r.outcome.as_str(), r.http_status),
            ("http-error", Some(503))
        );
        assert_eq!(
            bad_hits.load(Ordering::SeqCst),
            1,
            "exactly one request, no retry"
        );
        // A rerun in the same slot sends nothing at all.
        let again = ingest(&http, &bad_url, dir.path(), at(9)).await.unwrap();
        assert_eq!(again.outcome, "skipped-slot");
        assert_eq!(bad_hits.load(Ordering::SeqCst), 1);
        assert_eq!(fs::read(dir.path().join(ISS_FILE)).unwrap(), good);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(
            read_iss(dir.path()).unwrap().elements.norad_id,
            ISS_NORAD_ID
        );
    }

    #[tokio::test]
    async fn redirect_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let (url, hits) = server(301, "").await;
        let r = ingest(&client().unwrap(), &url, dir.path(), at(13))
            .await
            .unwrap();
        assert_eq!(r.http_status, Some(301));
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert!(read_iss(dir.path()).is_none());
    }

    #[tokio::test]
    async fn garbage_body_keeps_the_last_file() {
        let dir = tempfile::tempdir().unwrap();
        let (url, _) = server(200, "No GP data found").await;
        let r = ingest(&client().unwrap(), &url, dir.path(), at(19))
            .await
            .unwrap();
        assert_eq!(r.outcome, "invalid");
        assert!(read_iss(dir.path()).is_none());
    }
}
