use crate::{artifact::Artifact, catalogue::Catalogue, feed::Feed, health::Health, leases::Pool};
use anyhow::{Context, Result, ensure};
use chrono::Utc;
use clap::Args;
use object_store::aws::AmazonS3Builder;
use serde_json::json;
use std::time::{Duration, Instant};

#[derive(Args)]
pub struct TraceArgs {
    #[arg(long, default_value_t = 42.893889)]
    pub lat: f64,
    #[arg(long,default_value_t=-85.544889,allow_hyphen_values=true)]
    pub lon: f64,
    #[arg(long, default_value_t = 9)]
    pub zoom: u8,
    #[arg(long, default_value_t = 134)]
    pub x: u32,
    #[arg(long, default_value_t = 188)]
    pub y: u32,
}

fn required(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("{name} required"))
}

pub async fn run(args: TraceArgs) -> Result<()> {
    ensure!(
        radar_core::env::bool_var("LEVEL2_NUMERIC_ENABLED", false)?,
        "Level II numeric gate disabled"
    );
    ensure!(
        (8..=13).contains(&args.zoom) && args.x < 1u32 << args.zoom && args.y < 1u32 << args.zoom,
        "invalid tile demand"
    );
    let task = tokio::spawn(demand(args));
    match tokio::time::timeout(Duration::from_secs(30), task).await {
        Ok(result) => println!("{}", result??),
        Err(_) => println!(
            "{}",
            json!({"status":"fallback","reason":"pending_timeout","permits_retained":true})
        ),
    }
    Ok(())
}

async fn demand(args: TraceArgs) -> Result<serde_json::Value> {
    let start = Instant::now();
    let url = required("LEVEL2_PG_URL")?;
    let schema = std::env::var("LEVEL2_PG_SCHEMA").unwrap_or_else(|_| "level2_numeric".into());
    let mut pool = Pool::connect(&url, &schema).await?;
    pool.initialize().await?;
    let feed = Feed::new()?;
    let mut catalogue = pool.catalogue().await?.unwrap_or(Catalogue::snapshot()?);
    match catalogue.refresh(&feed.http, Utc::now()).await {
        Ok(diff) => {
            if !diff.is_empty() {
                eprintln!("catalogue_diff={}", serde_json::to_string(&diff)?);
            }
            pool.save_catalogue(&catalogue).await?;
        }
        Err(_) => eprintln!("catalogue_refresh_failed_using_last_good=true"),
    }
    let candidates = catalogue.candidates(args.lat, args.lon, &["KGRR".into()])?;
    if candidates.is_empty() {
        return Ok(json!({"status":"fallback","reason":"out_of_range"}));
    }
    let endpoint = required("LEVEL2_S3_ENDPOINT")?;
    let store = AmazonS3Builder::new()
        .with_endpoint(&endpoint)
        .with_allow_http(endpoint.starts_with("http://127.0.0.1:"))
        .with_bucket_name(required("LEVEL2_S3_BUCKET")?)
        .with_region("us-east-1")
        .with_access_key_id(required("LEVEL2_S3_ACCESS_KEY")?)
        .with_secret_access_key(required("LEVEL2_S3_SECRET_KEY")?)
        .build()?;
    let namespace = required("LEVEL2_OBJECT_PREFIX")?;
    let stations: Vec<_> = candidates.iter().map(|s| s.id.clone()).collect();
    let outcome = trace_candidates(&mut pool, &feed, &store, &stations, &namespace).await?;
    Ok(
        json!({"numeric":outcome,"runtime_ms":start.elapsed().as_millis(),"peak_rss_kib":peak_rss_kib()}),
    )
}

async fn trace_candidates(
    pool: &mut Pool,
    feed: &Feed,
    store: &dyn object_store::ObjectStore,
    stations: &[String],
    namespace: &str,
) -> Result<serde_json::Value> {
    // The request task retains persistent permits after its caller times out.
    let mut outcome = json!({"status":"fallback","reason":"station_saturated"});
    for station in stations.iter().take(3) {
        if !pool.station(station).await? {
            continue;
        }
        let Some(permit) = pool.decode().await? else {
            pool.release_station(station).await?;
            outcome = json!({"status":"fallback","reason":"decode_saturated"});
            break;
        };
        let result = trace(feed, pool, store, station, namespace).await;
        pool.release_decode(&permit).await?;
        pool.release_station(station).await?;
        outcome = match result {
            Ok(outcome) => outcome,
            Err(_) => {
                json!({"status":"fallback","reason":"artifact_unavailable","health":pool.health(station).await?.report(Utc::now())})
            }
        };
        if outcome["status"] == "numeric_ready" {
            break;
        }
    }
    Ok(outcome)
}

async fn trace(
    feed: &Feed,
    pool: &mut Pool,
    store: &dyn object_store::ObjectStore,
    station: &str,
    namespace: &str,
) -> Result<serde_json::Value> {
    let mut health = pool.health(station).await?;
    health.active = true;
    let latest = pool.latest(station).await?;
    let after = latest.as_ref().map(|(end, _, _)| *end);
    let acquisition = acquire(feed, station, &mut health, after).await;
    let acquired = match acquisition {
        Ok(acquired) => acquired,
        Err(_) => {
            health.access_failure = Some("source_access_failed".into());
            None
        }
    };
    if let Some(a) = &acquired {
        health.low_sweep_end = a
            .sweeps
            .iter()
            .map(|s| s.scan_end)
            .chain(health.low_sweep_end)
            .max();
    }
    pool.save_health(station, &health).await?;
    let report = health.report(Utc::now());
    if !report.eligible {
        return Ok(
            json!({"status":"fallback","reason":"low_sweep_ineligible","health":report,"clocks":health}),
        );
    }
    let Some(mut acquired) = acquired else {
        if let Some((_, identity, key)) = latest
            && crate::artifact::verify(store, &key).await.is_ok()
        {
            return Ok(
                json!({"status":"numeric_ready","cached":true,"public_frame_ready":false,"commit":key,"identity":identity,"health":report,"clocks":health}),
            );
        }
        return Ok(
            json!({"status":"fallback","reason":"no_verified_artifact","health":report,"clocks":health}),
        );
    };
    acquired.sweeps.sort_by_key(|s| s.scan_end);
    let sweep = acquired.sweeps.pop().context("no complete low sweep")?;
    let decoded_site = sweep.site.context("missing station geometry")?;
    let catalogue = pool.catalogue().await?.unwrap_or(Catalogue::snapshot()?);
    let site = catalogue
        .sites
        .get(station)
        .context("station absent from catalogue")?;
    ensure!(
        crate::catalogue::distance_m(
            site.lat,
            site.lon,
            f64::from(decoded_site.latitude_deg),
            f64::from(decoded_site.longitude_deg)
        ) <= 1500.0,
        "decoded station geometry disagrees with catalogue"
    );
    let artifact = Artifact::build(&sweep, acquired.sources, namespace)?;
    if let Some((end, identity, key)) = latest
        && sweep.scan_end == end
    {
        ensure!(identity == artifact.commit.identity, "timestamp collision");
        crate::artifact::verify(store, &key).await?;
        return Ok(
            json!({"status":"numeric_ready","cached":true,"public_frame_ready":false,"commit":key,"identity":identity,"health":report,"clocks":health}),
        );
    }
    pool.reserve_objects(&artifact.commit_key, artifact.bytes()?)
        .await?;
    artifact.publish(store).await?;
    pool.index(
        station,
        sweep.scan_end,
        &artifact.commit.identity,
        &artifact.commit_key,
    )
    .await?;
    Ok(
        json!({"status":"numeric_ready","public_frame_ready":false,"commit":artifact.commit_key,"identity":artifact.commit.identity,"source_bytes":artifact.commit.sources.iter().map(|s|s.bytes).sum::<usize>(),"sources":artifact.commit.sources,"rays":sweep.rays.len(),"scan_end":sweep.scan_end,"objects":artifact.commit.entries.len()+1,"artifact_bytes":artifact.bytes()?,"health":report,"clocks":health}),
    )
}

async fn acquire(
    feed: &Feed,
    station: &str,
    health: &mut Health,
    after: Option<chrono::DateTime<Utc>>,
) -> Result<Option<crate::feed::Acquisition>> {
    let hint = health
        .advancing_key
        .as_deref()
        .and_then(|key| key.split('/').nth(1))
        .and_then(|n| n.parse::<u16>().ok())
        .filter(|n| (1..=999).contains(n));
    let mut archives = None;
    let volume = if let Some(volume) = hint {
        volume
    } else {
        let candidates = feed.archive_candidates(station, Utc::now()).await?;
        let latest = candidates.first().context("no recovery archive")?;
        let header = feed.archive_header(latest).await?;
        let file = nexrad_data::volume::File::new(header);
        let number = file
            .header()
            .context("bad recovery header")?
            .extension_number()
            .context("missing volume")?
            .parse::<u16>()?;
        archives = Some(candidates);
        number
    };
    ensure!((1..=999).contains(&volume), "invalid circular volume");
    let mut acquired = None;
    for attempt in 0..3 {
        match feed.live_groups(station, volume).await {
            Ok(groups) => {
                health.access_failure = None;
                if let Some(newest) = groups
                    .iter()
                    .flatten()
                    .max_by_key(|o| (o.modified, o.key.clone()))
                {
                    health.observe(&newest.key, newest.modified, Utc::now());
                }
                for group in groups {
                    if let Ok(a) = feed.chunks(station, &group, health, after).await {
                        acquired = Some(a);
                        break;
                    }
                }
            }
            Err(_) => health.access_failure = Some("chunk_listing_failed".into()),
        }
        if acquired.is_some() {
            break;
        }
        if health.report(Utc::now()).eligible && after.is_some() {
            return Ok(None);
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    if acquired.is_none() {
        let archives = match archives {
            Some(candidates) => candidates,
            None => feed.archive_candidates(station, Utc::now()).await?,
        };
        for archive in archives {
            if let Ok(a) = feed.archive(station, &archive, health).await {
                acquired = Some(a);
                break;
            }
        }
    }
    Ok(acquired)
}

fn peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_candidate_falls_through_with_three_attempt_limit() -> Result<()> {
        let Ok(url) = std::env::var("LEVEL2_TEST_PG") else {
            eprintln!("SKIP: LEVEL2_TEST_PG absent");
            return Ok(());
        };
        let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
        let mut pool = Pool::connect(&url, &schema).await?;
        pool.initialize().await?;
        let feed = Feed {
            http: reqwest::Client::builder()
                .proxy(reqwest::Proxy::all("http://127.0.0.1:1")?)
                .build()?,
        };
        let stations = ["KGRR", "KDTX", "KLOT", "KAPX"].map(str::to_owned);
        let result = trace_candidates(
            &mut pool,
            &feed,
            &object_store::memory::InMemory::new(),
            &stations,
            "level2/test",
        )
        .await?;
        assert_eq!(result["status"], "fallback");
        assert_eq!(result["health"]["source_unreachable"], true);
        let count: i64 = pool
            .client
            .query_one(&format!("SELECT count(*) FROM {schema}.stations"), &[])
            .await?
            .get(0);
        assert_eq!(count, 3);
        assert!(
            pool.client
                .query_opt(
                    &format!("SELECT station FROM {schema}.stations WHERE station='KAPX'"),
                    &[]
                )
                .await?
                .is_none()
        );
        assert!(pool.decode().await?.is_some());
        Ok(())
    }
    #[tokio::test]
    #[ignore = "requires isolated PostgreSQL/RustFS credentials and anonymous NOAA access"]
    async fn real_kgrr_archive_recovery_when_chunk_access_fails() -> Result<()> {
        let url = required("LEVEL2_TEST_PG")?;
        let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
        let mut pool = Pool::connect(&url, &schema).await?;
        pool.initialize().await?;
        assert!(pool.station("KGRR").await?);
        let permit = pool.decode().await?.context("decode slot")?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .proxy(reqwest::Proxy::custom(|url| {
                (url.host_str() == Some("unidata-nexrad-level2-chunks.s3.amazonaws.com"))
                    .then_some("http://127.0.0.1:1")
            }))
            .build()?;
        let feed = Feed { http };
        let store = AmazonS3Builder::new()
            .with_endpoint(required("LEVEL2_S3_ENDPOINT")?)
            .with_allow_http(true)
            .with_bucket_name(required("LEVEL2_S3_BUCKET")?)
            .with_region("us-east-1")
            .with_access_key_id(required("LEVEL2_S3_ACCESS_KEY")?)
            .with_secret_access_key(required("LEVEL2_S3_SECRET_KEY")?)
            .build()?;
        let prefix = format!("level2/archive-test-{}", uuid::Uuid::new_v4().simple());
        let start = Instant::now();
        let result = trace(&feed, &mut pool, &store, "KGRR", &prefix).await?;
        pool.release_decode(&permit).await?;
        pool.release_station("KGRR").await?;
        println!(
            "archive_runtime_ms={} peak_rss_kib={:?} result={result}",
            start.elapsed().as_millis(),
            peak_rss_kib()
        );
        assert_eq!(result["status"], "numeric_ready");
        assert_eq!(result["sources"][0]["bucket"], "unidata-nexrad-level2");
        assert_eq!(result["health"]["source_unreachable"], true);
        Ok(())
    }
}
