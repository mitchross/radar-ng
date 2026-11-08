//! On-demand SPC watches and mesoscale discussions, with last-good fallback.

use std::time::{Duration, Instant};

use axum::{extract::State, http::StatusCode, response::Response};
use chrono::{DateTime, Datelike, Months, NaiveDate, TimeZone, Utc};
use serde_json::{Value, json};

use crate::{respond, state::AppState, upstream};

const TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Default)]
pub struct Snapshot {
    pub checked: Option<Instant>,
    pub body: Option<Value>,
    pub stale: bool,
}

pub async fn watches(State(state): State<AppState>) -> Response {
    products(state, false).await
}

pub async fn discussions(State(state): State<AppState>) -> Response {
    products(state, true).await
}

async fn products(state: AppState, md: bool) -> Response {
    let mut snapshot = state.spc_products[usize::from(md)].lock().await;
    if snapshot.checked.is_none_or(|t| t.elapsed() >= TTL) {
        let result = tokio::time::timeout(Duration::from_secs(25), fetch(&state, md)).await;
        snapshot.checked = Some(Instant::now());
        if let Ok(Ok(body)) = result {
            snapshot.body = Some(body);
            snapshot.stale = false;
        } else {
            snapshot.stale = true;
        }
    }
    let Some(mut body) = snapshot.body.clone() else {
        return respond::upstream_unavailable_no_store();
    };
    let now = Utc::now();
    if let Some(features) = body["features"].as_array_mut() {
        features.retain(|f| {
            f["properties"]["expires"]
                .as_str()
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .is_some_and(|t| t > now)
        });
    }
    body["stale"] = json!(snapshot.stale);
    // The in-process cache filters expiry on every request, including during upstream outages.
    respond::json_bytes(StatusCode::OK, respond::encode(&body), Some("no-store"))
}

async fn text(state: &AppState, path: &str) -> anyhow::Result<Option<String>> {
    let url = format!(
        "{}/{}",
        state.config.spc_products_base.trim_end_matches('/'),
        path
    );
    let fetched = upstream::get(
        &state.http,
        &url,
        &[],
        &[("User-Agent", state.config.nws_user_agent.as_str())],
        Duration::from_secs(8),
    )
    .await?;
    if fetched.status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    anyhow::ensure!(fetched.is_success(), "SPC HTTP {}", fetched.status);
    Ok(Some(String::from_utf8(fetched.body.to_vec())?))
}

async fn fetch(state: &AppState, md: bool) -> anyhow::Result<Value> {
    let dir = if md { "md" } else { "watch" };
    let index = text(state, &format!("{dir}/"))
        .await?
        .ok_or_else(|| anyhow::anyhow!("missing SPC index"))?;
    let ids = product_ids(&index, md)?;
    let mut features = Vec::new();
    for number in ids {
        let prefix = if md { "md" } else { "ww" };
        let path = format!("{dir}/{prefix}{number:04}.html");
        let html = text(state, &path)
            .await?
            .ok_or_else(|| anyhow::anyhow!("missing SPC product"))?;
        let mut feature = parse_product(&html, number, md)
            .ok_or_else(|| anyhow::anyhow!("invalid SPC product {path}"))?;
        if !md
            && let Some(probabilities) = text(state, &format!("watch/wwp{number:04}.txt")).await?
        {
            let rows: Vec<Value> = probabilities
                .lines()
                .filter_map(|line| {
                    let (label, value) = line.trim().split_once(':')?;
                    (label.starts_with("PROB OF ") && value.trim().ends_with('%'))
                        .then(|| json!({"label": label.trim(), "value": value.trim()}))
                })
                .collect();
            anyhow::ensure!(!rows.is_empty(), "invalid SPC probability product");
            feature["properties"]["probabilities"] = json!(rows);
        }
        features.push(feature);
    }
    Ok(
        json!({"type": "FeatureCollection", "features": features, "source": "NOAA/NWS Storm Prediction Center"}),
    )
}

fn product_ids(index: &str, md: bool) -> anyhow::Result<Vec<u32>> {
    let empty = if md {
        "No Mesoscale Discussions are currently in effect"
    } else {
        "No watches are currently valid"
    };
    if index.contains(empty) {
        return Ok(Vec::new());
    }
    let prefix = if md {
        "/products/md/md"
    } else {
        "/products/watch/ww"
    };
    let mut ids: Vec<u32> = index
        .split(prefix)
        .skip(1)
        .filter_map(|part| {
            let (digits, _) = part.split_once(".html")?;
            (digits.len() == 4 && digits.bytes().all(|c| c.is_ascii_digit()))
                .then(|| digits.parse().ok())
                .flatten()
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    anyhow::ensure!(!ids.is_empty() && ids.len() <= 64, "unrecognized SPC index");
    Ok(ids)
}

fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<'
            && chars
                .peek()
                .is_some_and(|next| next.is_ascii_alphabetic() || *next == '/')
        {
            for next in chars.by_ref() {
                if next == '>' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn blocks(html: &str) -> Vec<String> {
    html.split("<pre>")
        .skip(1)
        .filter_map(|p| {
            p.split_once("</pre>")
                .map(|(s, _)| plain(s).trim().to_owned())
        })
        .collect()
}

fn issued(text: &str) -> Option<DateTime<Utc>> {
    let line = text
        .lines()
        .find(|l| l.contains("NWS Storm Prediction Center Norman OK"))?;
    let rest = text.split_once(line)?.1;
    let words: Vec<&str> = rest.split_whitespace().take(7).collect();
    if words.len() != 7 {
        return None;
    }
    let clock: u32 = words[0].parse().ok()?;
    if !(1..=12).contains(&(clock / 100)) || clock % 100 >= 60 || !["AM", "PM"].contains(&words[1])
    {
        return None;
    }
    let hour = clock / 100 % 12 + if words[1] == "PM" { 12 } else { 0 };
    let offset = match words[2] {
        "EDT" => 4,
        "EST" | "CDT" => 5,
        "CST" | "MDT" => 6,
        "MST" | "PDT" => 7,
        "PST" => 8,
        "UTC" | "GMT" => 0,
        _ => return None,
    };
    let date = NaiveDate::parse_from_str(&words[4..].join(" "), "%b %e %Y").ok()?;
    let local = date.and_hms_opt(hour, clock % 100, 0)?;
    Some(Utc.from_utc_datetime(&local) + chrono::Duration::hours(offset))
}

fn utc_stamp(stamp: &str, reference: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let stamp = stamp.strip_suffix('Z')?;
    if stamp.len() != 6 || !stamp.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let day = stamp[..2].parse().ok()?;
    let hour = stamp[2..4].parse().ok()?;
    let minute = stamp[4..].parse().ok()?;
    [-1_i32, 0, 1]
        .into_iter()
        .filter_map(|delta| {
            let month = match delta {
                -1 => reference.checked_sub_months(Months::new(1))?,
                1 => reference.checked_add_months(Months::new(1))?,
                _ => reference,
            };
            Utc.with_ymd_and_hms(month.year(), month.month(), day, hour, minute, 0)
                .single()
        })
        .min_by_key(|dt| (*dt - reference).num_seconds().abs())
}

fn parse_product(html: &str, number: u32, md: bool) -> Option<Value> {
    let blocks = blocks(html);
    let title = if md {
        format!("Mesoscale Discussion {number:04}")
    } else {
        format!("Watch Number {number}")
    };
    let full = blocks
        .iter()
        .find(|b| b.lines().any(|line| line.trim().ends_with(&title)))?;
    let issued = issued(full)?;
    let coords = blocks.iter().find(|b| b.contains("LAT...LON"))?;
    let window = if md {
        full.lines().find_map(|l| l.trim().strip_prefix("Valid "))?
    } else {
        coords.lines().find(|l| l.trim().starts_with("WW "))?.trim()
    };
    let stamps: Vec<&str> = window
        .split_whitespace()
        .filter(|s| s.len() == 7 && s.ends_with('Z'))
        .collect();
    let start = utc_stamp(stamps.first()?, issued)?;
    let expires = utc_stamp(stamps.get(1)?, start)?;
    if expires <= start || (expires - start).num_hours() > 24 {
        return None;
    }
    let mut ring = Vec::new();
    for word in coords.split_once("LAT...LON")?.1.split_whitespace() {
        if word.len() != 8 || !word.bytes().all(|c| c.is_ascii_digit()) {
            break;
        }
        let lat = word[..4].parse::<f64>().ok()? / 100.0;
        let mut lon = word[4..].parse::<f64>().ok()? / 100.0;
        // SPC omits the leading 1 from longitudes west of 100 degrees.
        if lon < 30.0 {
            lon += 100.0;
        }
        if !(0.0..=90.0).contains(&lat) || lon > 180.0 {
            return None;
        }
        ring.push(json!([-lon, lat]));
    }
    if ring.len() < 3 {
        return None;
    }
    if ring.first() != ring.last() {
        ring.push(ring[0].clone());
    }
    let kind = if md {
        "md"
    } else if full.contains("Tornado Watch") {
        "tornado"
    } else if full.contains("Severe Thunderstorm Watch") {
        "severe"
    } else {
        return None;
    };
    let id = format!(
        "{}-{}-{number:04}",
        issued.year(),
        if md { "md" } else { "ww" }
    );
    Some(
        json!({"type": "Feature", "id": id, "geometry": {"type": "Polygon", "coordinates": [ring]},
        "properties": {"id": id, "number": number, "type": kind, "issued": issued.to_rfc3339(),
            "expires": expires.to_rfc3339(), "text": full, "probabilities": [],
            "source": "NOAA/NWS Storm Prediction Center", "geometry_note": if md { "SPC discussion area" } else { "SPC aviation watch approximation; official watch counties are in WOU" }}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_windows_cross_year_and_month_boundaries() {
        let end_year = Utc.with_ymd_and_hms(2026, 12, 31, 23, 0, 0).unwrap();
        assert_eq!(
            utc_stamp("010300Z", end_year).unwrap().to_rfc3339(),
            "2027-01-01T03:00:00+00:00"
        );
        let march = Utc.with_ymd_and_hms(2026, 3, 1, 0, 0, 0).unwrap();
        assert_eq!(utc_stamp("282300Z", march).unwrap().month(), 2);
        assert!(utc_stamp("000000Z", march).is_none());
        assert!(utc_stamp("あいうZ", march).is_none());
    }

    #[test]
    fn issuance_converts_local_noon_midnight_and_year_rollover_to_utc() {
        for (local, expected) in [
            ("1205 AM CST Thu Jan 1 2026", "2026-01-01T06:05:00+00:00"),
            ("1205 PM CDT Wed Jul 1 2026", "2026-07-01T17:05:00+00:00"),
            ("1130 PM PST Thu Dec 31 2026", "2027-01-01T07:30:00+00:00"),
        ] {
            let text = format!("NWS Storm Prediction Center Norman OK\n{local}\n");
            assert_eq!(issued(&text).unwrap().to_rfc3339(), expected);
        }
    }

    #[test]
    fn strips_links_without_losing_comparisons_or_probability_inequalities() {
        assert_eq!(
            plain(r#"Risk <5% and CAPE &lt;500 with <a href="x">WW 1</a>."#),
            "Risk <5% and CAPE <500 with WW 1."
        );
    }

    #[test]
    fn reads_zero_padded_discussion_titles() {
        let html = include_str!("../tests/fixtures/spc-md.html").replace("2448", "0099");
        let feature =
            parse_product(&html, 99, true).expect("SPC pads discussion numbers to four digits");
        assert_eq!(feature["properties"]["number"], 99);
        assert!(
            feature["properties"]["text"]
                .as_str()
                .unwrap()
                .contains("Mesoscale Discussion 0099")
        );
    }

    #[test]
    fn empty_indices_ignore_the_most_recent_expired_product_link() {
        assert!(
            product_ids(
                "No watches are currently valid <a href=\"/products/watch/ww0703.html\">Recent</a>",
                false
            )
            .unwrap()
            .is_empty()
        );
        assert!(
            product_ids("No Mesoscale Discussions are currently in effect", true)
                .unwrap()
                .is_empty()
        );
        assert!(product_ids("<html>maintenance</html>", true).is_err());
        assert_eq!(
            product_ids("/products/md/md2448.html /products/md/md2448.html", true).unwrap(),
            vec![2448]
        );
    }
}
