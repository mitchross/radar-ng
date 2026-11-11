use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, NaiveDateTime, SecondsFormat, Utc};
use radar_core::fsutil;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

const IEM: &str = "https://mesonet.agron.iastate.edu";
const MAX_RESPONSE: usize = 16 * 1024 * 1024;
const MAX_TOTAL: usize = 64 * 1024 * 1024;
const MAX_REQUESTS: usize = 512;
const MAX_VERSIONS: usize = 4096;
const HISTORY_DAYS: i64 = 7;

#[derive(Debug, Clone)]
pub struct Case {
    pub id: String,
    pub start: String,
    pub end: String,
    pub bounds: [f64; 4],
}

impl Case {
    pub fn named(id: &str) -> Result<Self> {
        let (start, end, bounds) = match id {
            "rolling-fork-2023" => (
                "2023-03-24T23:00:00Z",
                "2023-03-25T03:00:00Z",
                [-92., 31., -88., 35.],
            ),
            "greenfield-2024" => (
                "2024-05-21T19:00:00Z",
                "2024-05-21T22:00:00Z",
                [-97., 40., -92., 44.],
            ),
            _ => bail!("unknown replay case {id}"),
        };
        Ok(Self {
            id: id.into(),
            start: start.into(),
            end: end.into(),
            bounds,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Source {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Bulletin {
    pub url: String,
    pub sha256: String,
    pub text: String,
}

impl Bulletin {
    pub fn new(url: String, text: String) -> Self {
        Self {
            url,
            sha256: checksum(text.as_bytes()),
            text,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct History {
    pub r#type: String,
    pub schema_version: u32,
    pub case_id: String,
    pub start: String,
    pub end: String,
    pub bounds: [f64; 4],
    pub seed: Vec<Feature>,
    pub features: Vec<Feature>,
    pub bulletins: BTreeMap<String, Bulletin>,
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Feature {
    pub r#type: String,
    pub id: String,
    pub geometry: Value,
    pub properties: Properties,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Properties {
    pub year: i32,
    pub wfo: String,
    pub phenomena: String,
    pub significance: String,
    pub eventid: u32,
    pub event: String,
    pub status: String,
    pub product_id: String,
    pub utc_polygon_begin: String,
    pub utc_polygon_end: String,
    pub is_emergency: bool,
    pub is_pds: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct RawFeature {
    geometry: Value,
    properties: RawProperties,
}

#[derive(Debug, Clone, Deserialize)]
struct RawProperties {
    year: i32,
    wfo: String,
    phenomena: String,
    significance: String,
    eventid: u32,
    status: String,
    product_id: String,
    #[serde(alias = "polygon_begin")]
    utc_polygon_begin: String,
    #[serde(alias = "polygon_end")]
    utc_polygon_end: String,
    #[serde(default, alias = "issue")]
    utc_issue: Option<String>,
    #[serde(default, alias = "expire")]
    utc_expire: Option<String>,
}

impl RawProperties {
    fn key(&self) -> String {
        format!(
            "{}.{}.{}.{}.{}",
            self.year, self.wfo, self.phenomena, self.significance, self.eventid
        )
    }
}

fn checksum(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn timestamp(value: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("invalid warning timestamp {value}"))?
        .with_timezone(&Utc))
}

fn iso(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn supported(feature: &Value) -> bool {
    let p = &feature["properties"];
    p["significance"] == "W" && matches!(p["phenomena"].as_str(), Some("TO" | "SV" | "FF"))
}

fn raw(feature: &Value) -> Result<RawFeature> {
    let f: RawFeature =
        serde_json::from_value(feature.clone()).context("invalid IEM warning feature")?;
    let p = &f.properties;
    ensure!(
        p.wfo.len() == 3 && p.wfo.bytes().all(|b| b.is_ascii_uppercase()),
        "invalid warning office"
    );
    ensure!(p.eventid <= 9999, "invalid VTEC event number");
    ensure!(
        p.product_id.len() >= 28
            && p.product_id.len() <= 35
            && p.product_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid product id"
    );
    timestamp(&p.utc_polygon_begin)?;
    timestamp(&p.utc_polygon_end)?;
    Ok(f)
}

fn collection(value: &Value) -> Result<&Vec<Value>> {
    ensure!(
        value["type"] == "FeatureCollection",
        "IEM response is not a FeatureCollection"
    );
    let features = value["features"]
        .as_array()
        .context("IEM response has no features")?;
    ensure!(
        features.len() <= MAX_VERSIONS,
        "IEM feature budget exceeded"
    );
    Ok(features)
}

fn intersects(geometry: &Value, bounds: [f64; 4]) -> Result<bool> {
    let polygons = match geometry["type"].as_str() {
        Some("Polygon") => vec![&geometry["coordinates"]],
        Some("MultiPolygon") => geometry["coordinates"]
            .as_array()
            .context("invalid multipolygon")?
            .iter()
            .collect(),
        _ => bail!("warning geometry is not a polygon"),
    };
    let [west, south, east, north] = bounds;
    for polygon in polygons {
        let rings = polygon.as_array().context("invalid polygon rings")?;
        ensure!(!rings.is_empty(), "empty warning polygon");
        for ring in rings {
            let points = ring.as_array().context("invalid polygon ring")?;
            ensure!(
                points.len() >= 4 && points.first() == points.last(),
                "unclosed polygon ring"
            );
            let mut vertices = Vec::with_capacity(points.len());
            for point in points {
                let x = point[0].as_f64().context("invalid longitude")?;
                let y = point[1].as_f64().context("invalid latitude")?;
                ensure!(
                    (-180. ..=180.).contains(&x) && (-90. ..=90.).contains(&y),
                    "coordinate outside Earth"
                );
                vertices.push((x, y));
            }
            for edge in vertices.windows(2) {
                let (x, y) = edge[0];
                let (dx, dy) = (edge[1].0 - x, edge[1].1 - y);
                let (mut low, mut high) = (0.0_f64, 1.0_f64);
                for (p, q) in [
                    (-dx, x - west),
                    (dx, east - x),
                    (-dy, y - south),
                    (dy, north - y),
                ] {
                    if p == 0.0 {
                        if q < 0.0 {
                            high = -1.0;
                        }
                    } else if p < 0.0 {
                        low = low.max(q / p);
                    } else {
                        high = high.min(q / p);
                    }
                }
                if low <= high {
                    return Ok(true);
                }
            }
        }
    }
    let map = geometry.as_object().context("invalid geometry object")?;
    Ok([(west, south), (west, north), (east, south), (east, north)]
        .iter()
        .any(|&(x, y)| crate::alerts::point_in_geojson(x, y, map)))
}

fn resolve_bulletin(id: &str, response: &str) -> Result<String> {
    let fields: Vec<_> = id.split('-').collect();
    ensure!(
        (4..=5).contains(&fields.len()) && fields[0].len() == 12,
        "invalid bulletin identity {id}"
    );
    let mut matches = BTreeSet::new();
    for product in response.split('\u{3}') {
        let lines: Vec<_> = product.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let header: Vec<_> = line.split_whitespace().collect();
            if (3..=4).contains(&header.len())
                && header[0] == fields[2]
                && header[1] == fields[1]
                && header[2] == &fields[0][6..]
                && header.get(3).copied().unwrap_or("") == fields.get(4).copied().unwrap_or("")
                && lines
                    .get(i + 1)
                    .is_some_and(|line| line.trim() == fields[3])
            {
                let canonical = lines[i..].join("\n").trim().to_owned();
                matches.insert(canonical);
                break;
            }
        }
    }
    ensure!(
        matches.len() == 1,
        "bulletin {id} has {} exact WMO/AWIPS matches",
        matches.len()
    );
    Ok(matches.into_iter().next().expect("one matching bulletin"))
}

fn bulletin_labels(p: &RawProperties, bulletin: &Bulletin) -> Result<(bool, bool)> {
    let marker = format!(
        "/O.{}.K{}.{}.{}.{:04}.",
        p.status, p.wfo, p.phenomena, p.significance, p.eventid
    );
    let segments: Vec<_> = bulletin
        .text
        .split("$$")
        .filter(|s| s.contains(&marker))
        .collect();
    ensure!(
        segments.len() == 1,
        "bulletin {} has {} matching VTEC segments",
        p.product_id,
        segments.len()
    );
    let text = segments[0]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    let emergency = match p.phenomena.as_str() {
        "TO" => {
            text.contains("TORNADO EMERGENCY")
                || text.contains("TORNADO DAMAGE THREAT...CATASTROPHIC")
        }
        "FF" => {
            text.contains("FLASH FLOOD EMERGENCY")
                || text.contains("FLASH FLOOD DAMAGE THREAT...CATASTROPHIC")
        }
        _ => false,
    };
    Ok((emergency, text.contains("PARTICULARLY DANGEROUS SITUATION")))
}

pub fn normalize(
    case: &Case,
    seed: &Value,
    updates: &[Value],
    bulletins: BTreeMap<String, Bulletin>,
    sources: Vec<Source>,
) -> Result<History> {
    ensure!(
        updates.len() <= MAX_VERSIONS,
        "warning version budget exceeded"
    );
    let start = timestamp(&case.start)?;
    let end = timestamp(&case.end)?;
    let mut seeds = BTreeSet::new();
    for feature in collection(seed)?.iter().filter(|f| supported(f)) {
        let f = raw(feature)?;
        if intersects(&f.geometry, case.bounds)? {
            ensure!(
                timestamp(&f.properties.utc_polygon_begin)? <= start
                    && start < timestamp(&f.properties.utc_polygon_end)?,
                "seed is not active at case start"
            );
            seeds.insert(f.properties.key());
        }
    }
    let mut selected = seeds.clone();
    let mut rows = Vec::new();
    for feature in updates.iter().filter(|f| supported(f)) {
        let f = raw(feature)?;
        let begin = timestamp(&f.properties.utc_polygon_begin)?;
        let finish = timestamp(&f.properties.utc_polygon_end)?;
        if begin < end && finish >= start && intersects(&f.geometry, case.bounds)? {
            selected.insert(f.properties.key());
        }
        rows.push(f);
    }
    let mut resolved = BTreeMap::<(String, String), Feature>::new();
    let mut seen = BTreeMap::<String, Feature>::new();
    for f in rows
        .into_iter()
        .filter(|f| selected.contains(&f.properties.key()))
    {
        let p = &f.properties;
        let source_begin = timestamp(&p.utc_polygon_begin)?;
        let issued = NaiveDateTime::parse_from_str(&p.product_id[..12], "%Y%m%d%H%M")
            .context("invalid bulletin issuance time")?
            .and_utc();
        ensure!(
            p.status != "NEW" || issued <= source_begin,
            "incomplete corrected NEW history for {}: original polygon is unavailable",
            p.key()
        );
        let begin = source_begin.max(issued);
        let mut finish = timestamp(&p.utc_polygon_end)?;
        if let Some(expire) = &p.utc_expire {
            finish = finish.min(timestamp(expire)?);
        }
        ensure!(
            finish >= begin || p.status == "CAN",
            "negative warning validity"
        );
        if finish <= begin && p.status != "CAN" {
            continue;
        }
        ensure!(
            matches!(
                p.status.as_str(),
                "NEW" | "CON" | "CAN" | "EXP" | "EXT" | "EXA" | "EXB" | "COR"
            ),
            "unsupported VTEC status {}",
            p.status
        );
        let bulletin = bulletins
            .get(&p.product_id)
            .with_context(|| format!("missing bulletin {}", p.product_id))?;
        let (is_emergency, is_pds) = bulletin_labels(p, bulletin)?;
        let utc_polygon_begin = iso(begin);
        let id = format!("{}/{}/{}", p.key(), p.product_id, iso(source_begin));
        let feature = Feature {
            r#type: "Feature".into(),
            id,
            geometry: f.geometry,
            properties: Properties {
                year: p.year,
                wfo: p.wfo.clone(),
                phenomena: p.phenomena.clone(),
                significance: p.significance.clone(),
                eventid: p.eventid,
                event: match p.phenomena.as_str() {
                    "TO" => "Tornado Warning",
                    "SV" => "Severe Thunderstorm Warning",
                    _ => "Flash Flood Warning",
                }
                .into(),
                status: p.status.clone(),
                product_id: p.product_id.clone(),
                utc_polygon_begin: utc_polygon_begin.clone(),
                utc_polygon_end: iso(finish.max(begin)),
                is_emergency,
                is_pds,
            },
        };
        if let Some(previous) = seen.insert(feature.id.clone(), feature.clone()) {
            ensure!(
                previous == feature,
                "conflicting copies of warning version {}",
                feature.id
            );
        }
        let key = (p.key(), utc_polygon_begin);
        if let Some(previous) = resolved.get(&key) {
            // IEM product IDs append the WMO correction sequence, such as CCA or CCB.
            let rank = |f: &Feature| {
                (
                    f.properties.product_id.clone(),
                    f.properties.status == "CAN",
                    f.id.clone(),
                )
            };
            if rank(previous) > rank(&feature) {
                continue;
            }
            if rank(previous) == rank(&feature) {
                ensure!(
                    previous == &feature,
                    "conflicting copies of warning version {}",
                    feature.id
                );
                continue;
            }
        }
        resolved.insert(key, feature);
    }
    let mut features: Vec<Feature> = resolved.into_values().collect();
    for i in 0..features.len().saturating_sub(1) {
        let next = &features[i + 1].properties;
        let current = &features[i].properties;
        if (
            current.year,
            &current.wfo,
            &current.phenomena,
            current.eventid,
        ) == (next.year, &next.wfo, &next.phenomena, next.eventid)
            && current.utc_polygon_end > next.utc_polygon_begin
        {
            features[i].properties.utc_polygon_end = next.utc_polygon_begin.clone();
        }
    }
    features.retain(|f| {
        f.properties.status == "CAN"
            || f.properties.utc_polygon_begin < f.properties.utc_polygon_end
    });
    features.sort_by(|a, b| {
        let a = &a.properties;
        let b = &b.properties;
        (
            a.year,
            &a.wfo,
            a.eventid,
            &a.phenomena,
            &a.utc_polygon_begin,
        )
            .cmp(&(
                b.year,
                &b.wfo,
                b.eventid,
                &b.phenomena,
                &b.utc_polygon_begin,
            ))
    });
    let active: Vec<Feature> = features
        .iter()
        .filter(|f| {
            let p = &f.properties;
            p.status != "CAN" && p.utc_polygon_begin <= case.start && case.start < p.utc_polygon_end
        })
        .cloned()
        .collect();
    for key in seeds {
        ensure!(
            active.iter().any(|f| format!(
                "{}.{}.{}.{}.{}",
                f.properties.year,
                f.properties.wfo,
                f.properties.phenomena,
                f.properties.significance,
                f.properties.eventid
            ) == key),
            "missing active-at-start history for {key}"
        );
    }
    Ok(History {
        r#type: "FeatureCollection".into(),
        schema_version: 1,
        case_id: case.id.clone(),
        start: case.start.clone(),
        end: case.end.clone(),
        bounds: case.bounds,
        seed: active,
        features,
        bulletins,
        sources,
    })
}

struct Fetcher {
    client: reqwest::Client,
    bytes: usize,
    sources: Vec<Source>,
    base: String,
}

impl Fetcher {
    async fn get(&mut self, url: reqwest::Url) -> Result<Vec<u8>> {
        ensure!(
            self.sources.len() < MAX_REQUESTS,
            "IEM request budget exceeded"
        );
        let mut response = self
            .client
            .get(url.clone())
            .send()
            .await?
            .error_for_status()?;
        ensure!(
            response.status().is_success(),
            "unexpected IEM status {}",
            response.status()
        );
        ensure!(
            !response.headers().contains_key("X-IEM-Notice")
                || url
                    .query_pairs()
                    .any(|(key, value)| key == "nolimit" && value == "true"),
            "ambiguous IEM bulletin {url}"
        );
        ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= MAX_RESPONSE as u64),
            "IEM response too large"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= MAX_RESPONSE && self.bytes + chunk.len() <= MAX_TOTAL,
                "IEM byte budget exceeded"
            );
            self.bytes += chunk.len();
            bytes.extend_from_slice(&chunk);
        }
        self.sources.push(Source {
            url: url.into(),
            sha256: checksum(&bytes),
        });
        Ok(bytes)
    }

    async fn interval(
        &mut self,
        begin: &str,
        end: &str,
        office: Option<&str>,
        ph: Option<&str>,
    ) -> Result<Vec<Value>> {
        let mut url =
            reqwest::Url::parse(&format!("{}/api/1/vtec/sbw_interval.geojson", self.base))?;
        url.query_pairs_mut().extend_pairs([
            ("begints", begin),
            ("endts", end),
            ("only_new", "false"),
            ("include_can", "true"),
        ]);
        if let Some(office) = office {
            url.query_pairs_mut().append_pair("wfo", office);
        }
        if let Some(ph) = ph {
            url.query_pairs_mut().append_pair("ph", ph);
        }
        let value: Value = serde_json::from_slice(&self.get(url).await?)?;
        Ok(collection(&value)?.clone())
    }
}

pub async fn import(case_id: &str, output: &Path) -> Result<History> {
    import_from(case_id, output, IEM).await
}

async fn import_from(case_id: &str, output: &Path, base: &str) -> Result<History> {
    let case = Case::named(case_id)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(45))
        .connect_timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("radar-ng historical warning import")
        .build()?;
    let mut fetcher = Fetcher {
        client,
        bytes: 0,
        sources: vec![],
        base: base.into(),
    };
    let mut url = reqwest::Url::parse(&format!("{base}/geojson/sbw.geojson"))?;
    url.query_pairs_mut().append_pair("ts", &case.start);
    let seed: Value = serde_json::from_slice(&fetcher.get(url).await?)?;
    let mut versions = fetcher.interval(&case.start, &case.end, None, None).await?;
    let mut seed_events = BTreeMap::new();
    for feature in collection(&seed)?.iter().filter(|f| supported(f)) {
        let f = raw(feature)?;
        if intersects(&f.geometry, case.bounds)? {
            seed_events.insert(f.properties.key(), f.properties);
        }
    }
    ensure!(seed_events.len() <= 32, "seed event budget exceeded");
    for (key, p) in &seed_events {
        let issue = timestamp(
            p.utc_issue
                .as_deref()
                .context("seed has no event issuance")?,
        )?;
        let horizon = issue + chrono::Duration::days(HISTORY_DAYS);
        ensure!(
            issue <= timestamp(&case.start)? && horizon >= timestamp(&case.end)?,
            "seed history exceeds seven-day budget"
        );
        let history = fetcher
            .interval(&iso(issue), &iso(horizon), Some(&p.wfo), Some(&p.phenomena))
            .await?;
        let mut found_new = false;
        for feature in history.iter().filter(|f| supported(f)) {
            let f = raw(feature)?;
            if f.properties.key() == *key {
                let expire = timestamp(
                    f.properties
                        .utc_expire
                        .as_deref()
                        .unwrap_or(&f.properties.utc_polygon_end),
                )?;
                ensure!(
                    expire < horizon,
                    "seed history may continue beyond bounded query"
                );
                found_new |= f.properties.status == "NEW";
                versions.push(feature.clone());
            }
        }
        ensure!(found_new, "seed history has no issuance for {key}");
    }
    ensure!(
        versions.len() <= MAX_VERSIONS,
        "warning version budget exceeded"
    );
    let mut relevant = BTreeSet::new();
    for feature in versions.iter().filter(|f| supported(f)) {
        let f = raw(feature)?;
        if intersects(&f.geometry, case.bounds)? {
            relevant.insert(f.properties.key());
        }
    }
    versions
        .retain(|f| supported(f) && raw(f).is_ok_and(|f| relevant.contains(&f.properties.key())));
    let products: BTreeSet<String> = versions
        .iter()
        .map(|f| raw(f).map(|f| f.properties.product_id))
        .collect::<Result<_>>()?;
    let mut bulletins = BTreeMap::new();
    for id in products {
        let url = format!("{base}/api/1/nwstext/{id}?nolimit=true");
        let text = String::from_utf8(fetcher.get(reqwest::Url::parse(&url)?).await?)?;
        let text = resolve_bulletin(&id, &text)?;
        bulletins.insert(id, Bulletin::new(url, text));
    }
    let history = normalize(&case, &seed, &versions, bulletins, fetcher.sources)?;
    let bytes = serde_json::to_vec(&history)?;
    ensure!(bytes.len() <= MAX_TOTAL, "warning cache budget exceeded");
    fsutil::atomic_write(output, &bytes, ".replay-warnings-", ".tmp")?;
    Ok(history)
}

/// Largest warnings file the read-only API serves; a bigger file would silently never list.
pub const MAX_PUBLIC_BYTES: usize = 16 * 1024 * 1024;

/// Check a cached warnings file for `case_id` before it is published. Fails closed.
pub fn validate_file(case_id: &str, bytes: &[u8]) -> Result<History> {
    ensure!(
        bytes.len() <= MAX_PUBLIC_BYTES,
        "warnings file is {} bytes; the API serves at most {MAX_PUBLIC_BYTES}",
        bytes.len()
    );
    let case = Case::named(case_id)?;
    let history: History =
        serde_json::from_slice(bytes).context("warnings file is not a replay history")?;
    ensure!(
        history.r#type == "FeatureCollection" && history.schema_version == 1,
        "warnings file is not a schema 1 FeatureCollection"
    );
    ensure!(
        history.case_id == case.id
            && history.start == case.start
            && history.end == case.end
            && history.bounds == case.bounds,
        "warnings file is for a different case or window than {case_id}"
    );
    ensure!(!history.sources.is_empty(), "warnings file has no sources");
    for source in &history.sources {
        ensure!(
            source.url.starts_with("https://mesonet.agron.iastate.edu/")
                && source.sha256.len() == 64
                && source.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid warnings source {}",
            source.url
        );
    }
    ensure!(
        !history.features.is_empty(),
        "warnings file has no warning versions"
    );
    for (id, bulletin) in &history.bulletins {
        ensure!(
            bulletin.sha256 == checksum(bulletin.text.as_bytes()),
            "bulletin {id} hash does not match its text"
        );
    }
    let start = timestamp(&case.start)?;
    let end = timestamp(&case.end)?;
    let mut ids = BTreeSet::new();
    let mut overlaps = false;
    for feature in &history.features {
        let p = &feature.properties;
        ensure!(
            feature.r#type == "Feature",
            "{} is not a Feature",
            feature.id
        );
        ensure!(
            ids.insert(feature.id.as_str()),
            "duplicate feature {}",
            feature.id
        );
        ensure!(
            p.significance == "W" && matches!(p.phenomena.as_str(), "TO" | "SV" | "FF"),
            "unsupported warning type in {}",
            feature.id
        );
        ensure!(
            p.wfo.len() == 3 && p.wfo.bytes().all(|b| b.is_ascii_uppercase()) && p.eventid <= 9999,
            "invalid warning identity in {}",
            feature.id
        );
        ensure!(
            matches!(
                p.status.as_str(),
                "NEW" | "CON" | "CAN" | "EXP" | "EXT" | "EXA" | "EXB" | "COR"
            ),
            "unsupported VTEC status {} in {}",
            p.status,
            feature.id
        );
        let event = match p.phenomena.as_str() {
            "TO" => "Tornado Warning",
            "SV" => "Severe Thunderstorm Warning",
            _ => "Flash Flood Warning",
        };
        ensure!(p.event == event, "wrong event name in {}", feature.id);
        let begin = timestamp(&p.utc_polygon_begin)?;
        let finish = timestamp(&p.utc_polygon_end)?;
        ensure!(
            begin < finish || (p.status == "CAN" && begin == finish),
            "empty or negative validity in {}",
            feature.id
        );
        let key = format!(
            "{}.{}.{}.{}.{}",
            p.year, p.wfo, p.phenomena, p.significance, p.eventid
        );
        ensure!(
            feature.id.starts_with(&format!("{key}/{}/", p.product_id)),
            "feature id does not match its identity: {}",
            feature.id
        );
        intersects(&feature.geometry, [-180., -90., 180., 90.])
            .with_context(|| format!("bad geometry in {}", feature.id))?;
        let bulletin = history
            .bulletins
            .get(&p.product_id)
            .with_context(|| format!("missing bulletin {}", p.product_id))?;
        let raw = RawProperties {
            year: p.year,
            wfo: p.wfo.clone(),
            phenomena: p.phenomena.clone(),
            significance: p.significance.clone(),
            eventid: p.eventid,
            status: p.status.clone(),
            product_id: p.product_id.clone(),
            utc_polygon_begin: p.utc_polygon_begin.clone(),
            utc_polygon_end: p.utc_polygon_end.clone(),
            utc_issue: None,
            utc_expire: None,
        };
        ensure!(
            bulletin_labels(&raw, bulletin)? == (p.is_emergency, p.is_pds),
            "emergency/PDS flags in {} disagree with its bulletin",
            feature.id
        );
        if p.status != "CAN"
            && begin < end
            && finish > start
            && intersects(&feature.geometry, case.bounds)?
        {
            overlaps = true;
        }
    }
    ensure!(overlaps, "no warning overlaps the case window and bounds");
    for seed in &history.seed {
        let p = &seed.properties;
        ensure!(
            history.features.contains(seed)
                && p.status != "CAN"
                && timestamp(&p.utc_polygon_begin)? <= start
                && start < timestamp(&p.utc_polygon_end)?,
            "seed {} is not an active-at-start version",
            seed.id
        );
    }
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::{Path as AxumPath, Query, State},
        http::StatusCode,
        response::IntoResponse,
        routing::get,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Mock {
        queries: Arc<Mutex<Vec<BTreeMap<String, String>>>>,
        fail: bool,
    }

    fn feature() -> Value {
        json!({"type":"Feature", "geometry":{"type":"Polygon","coordinates":[[[-91.,32.],[-90.,32.],[-90.,33.],[-91.,33.],[-91.,32.]]]},
            "properties":{"year":2023,"wfo":"JAN","phenomena":"TO","significance":"W","eventid":1,"status":"NEW",
            "product_id":"202303242250-KJAN-WFUS54-TORJAN", "utc_issue":"2023-03-24T22:50:00Z","utc_expire":"2023-03-25T00:30:00Z",
            "utc_polygon_begin":"2023-03-24T22:50:00Z","utc_polygon_end":"2023-03-25T00:30:00Z"}})
    }

    async fn seed() -> Json<Value> {
        let mut f = feature();
        let p = f["properties"].as_object_mut().unwrap();
        for (old, new) in [
            ("utc_polygon_begin", "polygon_begin"),
            ("utc_polygon_end", "polygon_end"),
            ("utc_issue", "issue"),
            ("utc_expire", "expire"),
        ] {
            let value = p.remove(old).unwrap();
            p.insert(new.into(), value);
        }
        Json(json!({"type":"FeatureCollection","features":[f]}))
    }

    async fn interval(
        State(mock): State<Mock>,
        Query(query): Query<BTreeMap<String, String>>,
    ) -> Json<Value> {
        mock.queries.lock().unwrap().push(query.clone());
        let features = if query.contains_key("wfo") {
            vec![feature()]
        } else {
            vec![]
        };
        Json(json!({"type":"FeatureCollection","features":features}))
    }

    async fn text(State(mock): State<Mock>, AxumPath(id): AxumPath<String>) -> impl IntoResponse {
        assert_eq!(id, "202303242250-KJAN-WFUS54-TORJAN");
        if mock.fail {
            return (StatusCode::SERVICE_UNAVAILABLE, "unavailable");
        }
        (
            StatusCode::OK,
            "WFUS54 KJAN 242250\nTORJAN\n/O.NEW.KJAN.TO.W.0001.230324T2250Z-230325T0030Z/\nordinary\n$$",
        )
    }

    #[test]
    fn bulletin_bundles_match_exact_original_repeat_and_correction_headers() {
        let original = "WWUS53 KOAX 211853\nSVSOAX\noriginal";
        let repeat = "WWUS53 KOAX 211853 RRA\nSVSOAX\nrepeat";
        let correction = "WWUS53 KOAX 211853 CCA\nSVSOAX\ncorrection";
        let response = format!("935\n{repeat}\u{3}322\n{original}\u{3}{correction}\u{3}{original}");
        for (suffix, expected) in [("", original), ("-RRA", repeat), ("-CCA", correction)] {
            let id = format!("202405211853-KOAX-WWUS53-SVSOAX{suffix}");
            assert_eq!(resolve_bulletin(&id, &response).unwrap(), expected);
        }
        let conflict = format!("{original}\u{3}{original} changed");
        assert!(resolve_bulletin("202405211853-KOAX-WWUS53-SVSOAX", &conflict).is_err());
        assert!(resolve_bulletin("202405211853-KOAX-WWUS53-SVSOAX-CCB", &response).is_err());
    }

    #[tokio::test]
    async fn import_recovers_seed_history_and_publishes_only_after_success() {
        for fail in [false, true] {
            let mock = Mock {
                fail,
                ..Mock::default()
            };
            let app = Router::new()
                .route("/geojson/sbw.geojson", get(seed))
                .route("/api/1/vtec/sbw_interval.geojson", get(interval))
                .route("/api/1/nwstext/{id}", get(text))
                .with_state(mock.clone());
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("warnings.geojson");
            std::fs::write(&output, b"prior revision").unwrap();
            let result = import_from("rolling-fork-2023", &output, &base).await;
            if fail {
                assert!(result.is_err());
                assert_eq!(std::fs::read(&output).unwrap(), b"prior revision");
            } else {
                let history = result.unwrap();
                assert_eq!(history.seed.len(), 1);
                assert_eq!(history.features.len(), 1);
                assert_eq!(history.sources.len(), 4);
                assert_eq!(history.bulletins.len(), 1);
                assert_eq!(
                    std::fs::read(&output).unwrap(),
                    serde_json::to_vec(&history).unwrap()
                );
                assert!(history.sources.iter().all(|s| s.sha256.len() == 64));
            }
            let queries = mock.queries.lock().unwrap();
            assert_eq!(queries.len(), 2);
            assert!(
                queries
                    .iter()
                    .all(|q| q["only_new"] == "false" && q["include_can"] == "true")
            );
            assert_eq!(queries[1]["begints"], "2023-03-24T22:50:00Z");
            assert_eq!(queries[1]["endts"], "2023-03-31T22:50:00Z");
            server.abort();
        }
    }

    #[tokio::test]
    async fn fetch_budgets_and_ambiguous_bulletins_fail_closed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let app = Router::new()
            .route(
                "/",
                get(|| async { ([("X-IEM-Notice", "ambiguous")], "text") }),
            )
            .route("/bytes", get(|| async { "data" }))
            .route("/large", get(|| async { "x".repeat(MAX_RESPONSE + 1) }))
            .route("/redirect", get(|| async { StatusCode::FOUND }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut fetcher = Fetcher {
            client: reqwest::Client::new(),
            bytes: 0,
            sources: vec![],
            base: url.clone(),
        };
        assert!(
            fetcher
                .get(reqwest::Url::parse(&url).unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
        assert!(
            fetcher
                .get(reqwest::Url::parse(&format!("{url}large")).unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        fetcher.bytes = MAX_TOTAL;
        assert!(
            fetcher
                .get(reqwest::Url::parse(&format!("{url}bytes")).unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("byte budget")
        );
        assert!(
            fetcher
                .get(reqwest::Url::parse(&format!("{url}redirect")).unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("unexpected IEM status")
        );
        fetcher.sources.resize(
            MAX_REQUESTS,
            Source {
                url: url.clone(),
                sha256: String::new(),
            },
        );
        assert!(
            fetcher
                .get(reqwest::Url::parse(&url).unwrap())
                .await
                .unwrap_err()
                .to_string()
                .contains("request budget")
        );
        server.abort();
    }

    #[test]
    fn regional_geometry_checks_edges_containment_and_holes() {
        let geometry = &feature()["geometry"];
        assert!(intersects(geometry, [-90.5, 32.5, -89., 34.]).unwrap());
        assert!(intersects(geometry, [-90.9, 32.1, -90.1, 32.9]).unwrap());
        assert!(!intersects(geometry, [-89., 32., -88., 33.]).unwrap());
        let diamond = json!({"type":"Polygon","coordinates":[[[0,1],[1,0],[0,-1],[-1,0],[0,1]]]});
        assert!(!intersects(&diamond, [0.8, 0.8, 1., 1.]).unwrap());
        let hole = json!({"type":"Polygon","coordinates":[[[-2,-2],[2,-2],[2,2],[-2,2],[-2,-2]],[[-1,-1],[-1,1],[1,1],[1,-1],[-1,-1]]]});
        assert!(!intersects(&hole, [-0.5, -0.5, 0.5, 0.5]).unwrap());
        assert!(Case::named("../live").is_err());
    }
}
