//! Operations-gate harness for the isolated Level II pool. Needs Postgres in `LEVEL2_TEST_PG`.
//! `LEVEL2_HARNESS_REQUIRE_PG=1` turns a missing database into a failure instead of a skip.

use anyhow::Result;
use object_store::{ObjectStore, PutPayload, memory::InMemory, path::Path};
use radar_level2::{feed::Feed, leases::Pool, metrics, poller::Poller};
use std::sync::Arc;

const NS: &str = "level2/harness";

fn database() -> Option<String> {
    match std::env::var("LEVEL2_TEST_PG") {
        Ok(url) => Some(url),
        Err(_) if std::env::var("LEVEL2_HARNESS_REQUIRE_PG").is_ok_and(|v| v == "1") => {
            panic!("LEVEL2_HARNESS_REQUIRE_PG=1 but LEVEL2_TEST_PG is unset")
        }
        Err(_) => {
            eprintln!("SKIP: LEVEL2_TEST_PG absent");
            None
        }
    }
}

fn schema() -> String {
    format!("level2_test_{}", uuid::Uuid::new_v4().simple())
}

async fn replica(url: &str, schema: &str) -> Result<Pool> {
    let pool = Pool::connect(url, schema).await?;
    pool.initialize().await?;
    Ok(pool)
}

async fn connect(url: &str, schema: &str) -> Result<Pool> {
    Pool::connect(url, schema).await
}

async fn put(store: &dyn ObjectStore, key: &str) -> Result<()> {
    store
        .put(&Path::from(key), PutPayload::from(vec![7u8; 16]))
        .await?;
    Ok(())
}

async fn exists(store: &dyn ObjectStore, key: &str) -> bool {
    store.head(&Path::from(key)).await.is_ok()
}

/// Moves a station's demand expiry to `seconds_ago` in the past.
async fn expire_demand(pool: &Pool, station: &str, seconds_ago: i64) -> Result<()> {
    pool.client
        .execute(
            &format!(
                "UPDATE {}.stations SET demand_until=clock_timestamp()-make_interval(secs=>$2) WHERE station=$1",
                pool.schema()
            ),
            &[&station, &(seconds_ago as f64)],
        )
        .await?;
    Ok(())
}

fn poller(url: &str, schema: &str, store: Arc<dyn ObjectStore>) -> Result<Poller> {
    Poller::new(url.into(), schema.into(), store, NS.into())
}

async fn count(pool: &Pool, sql: &str) -> Result<i64> {
    Ok(pool
        .client
        .query_one(&sql.replace("{s}", pool.schema()), &[])
        .await?
        .get(0))
}

#[tokio::test]
async fn eight_stations_and_two_decodes_hold_across_replicas_and_rollout_overlap() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut old = replica(&url, &schema).await?;
    let mut new_a = replica(&url, &schema).await?;
    let mut new_b = replica(&url, &schema).await?;
    // Three overlapping replicas race twelve stations each.
    let attempts = (0..36).map(|n| {
        let (url, schema) = (url.clone(), schema.clone());
        async move {
            let mut pool = connect(&url, &schema).await?;
            pool.station(&format!("S{}", n % 12)).await
        }
    });
    let granted = futures_util::future::join_all(attempts)
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    assert!(granted.iter().any(|g| *g));
    assert_eq!(count(&old, "SELECT count(*) FROM {s}.stations").await?, 8);
    assert!(!old.station("S99").await?);
    // Rollout overlap: old takes one decode, the new ReplicaSet one, nobody gets a third.
    let first = old.decode().await?.expect("first permit");
    let second = new_a.decode().await?.expect("second permit");
    assert!(new_b.decode().await?.is_none());
    assert_eq!(old.usage().await?.decodes_running, 2);
    // The old pod dies without releasing: its lease is reaped only after the stale window.
    assert_eq!(new_b.reap().await?.decodes, 0);
    assert!(new_b.decode().await?.is_none());
    new_b
        .client
        .execute(
            &format!("UPDATE {schema}.decodes SET leased_at=clock_timestamp()-interval '16 minutes' WHERE owner=$1"),
            &[&first],
        )
        .await?;
    assert_eq!(new_b.reap().await?.decodes, 1);
    let third = new_b.decode().await?.expect("permit after reap");
    assert_eq!(new_b.usage().await?.decodes_running, 2);
    new_a.release_decode(&second).await?;
    new_b.release_decode(&third).await?;
    assert_eq!(new_b.usage().await?.decodes_running, 0);
    Ok(())
}

#[tokio::test]
async fn polling_claims_each_station_once_and_never_extends_demand() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut seed = replica(&url, &schema).await?;
    for n in 0..8 {
        assert!(seed.station(&format!("S{n}")).await?);
        seed.release_station(&format!("S{n}")).await?;
    }
    let before: Vec<chrono::DateTime<chrono::Utc>> = seed
        .client
        .query(
            &format!("SELECT demand_until FROM {schema}.stations ORDER BY station"),
            &[],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(seed.demanded().await?.len(), 8);
    let claims = (0..24).map(|n| {
        let (url, schema) = (url.clone(), schema.clone());
        async move {
            let mut pool = connect(&url, &schema).await?;
            pool.claim_poll(&format!("S{}", n % 8)).await
        }
    });
    let won = futures_util::future::join_all(claims)
        .await
        .into_iter()
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|w| *w)
        .count();
    assert_eq!(won, 8);
    let after: Vec<chrono::DateTime<chrono::Utc>> = seed
        .client
        .query(
            &format!("SELECT demand_until FROM {schema}.stations ORDER BY station"),
            &[],
        )
        .await?
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(before, after);
    Ok(())
}

#[tokio::test]
async fn idle_after_ten_minutes_then_evicted_after_thirty_unless_pinned() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut pool = replica(&url, &schema).await?;
    for s in ["KGRR", "KDTX"] {
        assert!(pool.station(s).await?);
        pool.release_station(s).await?;
    }
    let left: f64 = pool
        .client
        .query_one(&format!("SELECT extract(epoch FROM demand_until-clock_timestamp())::float8 FROM {schema}.stations WHERE station='KGRR'"), &[])
        .await?
        .get(0);
    assert!(
        (590.0..=600.0).contains(&left),
        "grant holds demand for ten minutes, got {left}"
    );
    assert_eq!(pool.demanded().await?, ["KDTX", "KGRR"]);
    expire_demand(&pool, "KGRR", 1).await?;
    assert_eq!(pool.demanded().await?, ["KDTX"]);
    assert!(
        pool.idle_stations().await?.claimed.is_empty(),
        "idle but inside the 30 min window"
    );
    expire_demand(&pool, "KGRR", 1799).await?;
    assert!(pool.idle_stations().await?.claimed.is_empty());
    expire_demand(&pool, "KGRR", 1801).await?;
    expire_demand(&pool, "KDTX", 1801).await?;
    pool.client
        .execute(
            &format!("INSERT INTO {schema}.pins(key,owner,until) VALUES($1,'view',clock_timestamp()+interval '60 seconds')"),
            &[&format!("{NS}/tiles/KDTX/id/v1/classic/9/1/1.png")],
        )
        .await?;
    let idle = pool.idle_stations().await?;
    assert_eq!(idle.claimed, ["KGRR"]);
    assert_eq!(idle.pinned, ["KDTX"]);
    Ok(())
}

#[tokio::test]
async fn tick_evicts_idle_station_data_and_keeps_pinned_and_demanded_stations() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let store = Arc::new(InMemory::new());
    let mut pool = replica(&url, &schema).await?;
    for s in ["KGRR", "KDTX", "KILN"] {
        assert!(pool.station(s).await?);
        pool.release_station(s).await?;
        let now = chrono::Utc::now();
        let key = format!("{NS}/sweeps/{s}/id1/r1/commit.json");
        put(store.as_ref(), &key).await?;
        put(
            store.as_ref(),
            &format!("{NS}/tiles/{s}/id1/v1/classic/9/1/1.png"),
        )
        .await?;
        pool.reserve_objects(&key, 16).await?;
        pool.index(s, now, "id1", &key).await?;
    }
    expire_demand(&pool, "KGRR", 1801).await?;
    expire_demand(&pool, "KDTX", 1801).await?;
    pool.client
        .execute(
            &format!("INSERT INTO {schema}.pins(key,owner,until) VALUES($1,'view',clock_timestamp()+interval '60 seconds')"),
            &[&format!("{NS}/tiles/KDTX/id1/v1/classic/9/1/1.png")],
        )
        .await?;
    let tick = poller(&url, &schema, store.clone())?.tick().await?;
    assert_eq!(tick.stations, ["KILN"]);
    assert!(
        !exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KGRR/id1/r1/commit.json")
        )
        .await
    );
    assert!(
        !exists(
            store.as_ref(),
            &format!("{NS}/tiles/KGRR/id1/v1/classic/9/1/1.png")
        )
        .await
    );
    assert!(
        exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KDTX/id1/r1/commit.json")
        )
        .await
    );
    assert!(
        exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KILN/id1/r1/commit.json")
        )
        .await
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.stations WHERE station='KGRR'"
        )
        .await?,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.sweeps WHERE station='KGRR'"
        )
        .await?,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.objects WHERE id LIKE '%/KGRR/%'"
        )
        .await?,
        0
    );
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.sweeps WHERE station='KDTX'"
        )
        .await?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn six_sweeps_stay_indexed_and_older_ones_go_unless_a_view_pins_them() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let store = Arc::new(InMemory::new());
    let mut pool = replica(&url, &schema).await?;
    assert!(pool.station("KGRR").await?);
    pool.release_station("KGRR").await?;
    let base = chrono::Utc::now() - chrono::TimeDelta::minutes(20);
    for n in 0..8 {
        let key = format!("{NS}/sweeps/KGRR/id{n}/r1/commit.json");
        put(store.as_ref(), &key).await?;
        put(store.as_ref(), &format!("{NS}/sweeps/KGRR/id{n}/r1/REF/0")).await?;
        pool.reserve_objects(&key, 16).await?;
        pool.index(
            "KGRR",
            base + chrono::TimeDelta::minutes(n),
            &format!("id{n}"),
            &key,
        )
        .await?;
    }
    pool.client
        .execute(
            &format!("INSERT INTO {schema}.pins(key,owner,until) VALUES($1,'view',clock_timestamp()+interval '60 seconds')"),
            &[&format!("{NS}/tiles/KGRR/id0/v1/classic/9/1/1.png")],
        )
        .await?;
    poller(&url, &schema, store.clone())?.tick().await?;
    assert_eq!(
        count(&pool, "SELECT count(*) FROM {s}.sweeps WHERE indexed").await?,
        6
    );
    assert!(
        exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KGRR/id0/r1/commit.json")
        )
        .await,
        "pinned sweep survives"
    );
    assert!(
        !exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KGRR/id1/r1/commit.json")
        )
        .await
    );
    assert!(!exists(store.as_ref(), &format!("{NS}/sweeps/KGRR/id1/r1/REF/0")).await);
    assert!(
        exists(
            store.as_ref(),
            &format!("{NS}/sweeps/KGRR/id7/r1/commit.json")
        )
        .await
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM {s}.sweeps").await?, 7);
    Ok(())
}

#[tokio::test]
async fn object_store_outage_keeps_idle_data_and_releases_the_eviction_claim() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let down: Arc<dyn ObjectStore> = Arc::new(
        object_store::aws::AmazonS3Builder::new()
            .with_endpoint("http://127.0.0.1:1")
            .with_allow_http(true)
            .with_bucket_name("down")
            .with_region("us-east-1")
            .with_access_key_id("k")
            .with_secret_access_key("s")
            .with_retry(object_store::RetryConfig {
                max_retries: 0,
                retry_timeout: std::time::Duration::from_secs(1),
                backoff: Default::default(),
            })
            .build()?,
    );
    let mut pool = replica(&url, &schema).await?;
    assert!(pool.station("KGRR").await?);
    pool.release_station("KGRR").await?;
    expire_demand(&pool, "KGRR", 1801).await?;
    let before = metrics::counter("radar_level2_evict_total", &[("result", "failed")]);
    poller(&url, &schema, down)?.tick().await?;
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.stations WHERE station='KGRR' AND NOT busy"
        )
        .await?,
        1
    );
    assert_eq!(
        metrics::counter("radar_level2_evict_total", &[("result", "failed")]),
        before + 1.0
    );
    Ok(())
}

#[tokio::test]
async fn noaa_outage_falls_back_cleanly_and_frees_every_permit() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut pool = replica(&url, &schema).await?;
    assert!(pool.station("KGRR").await?);
    pool.release_station("KGRR").await?;
    let mut feed = Feed::new()?;
    feed.http = reqwest::Client::builder()
        .proxy(reqwest::Proxy::all("http://127.0.0.1:1")?)
        .build()?;
    let poller = poller(&url, &schema, Arc::new(InMemory::new()))?.with_feed(feed);
    let started = metrics::hist_count("radar_level2_permit_wait_seconds");
    let outcome = poller.poll("KGRR").await?;
    assert_eq!(outcome.status, "fallback");
    assert_eq!(outcome.reason.as_deref(), Some("low_sweep_ineligible"));
    assert!(
        metrics::counter(
            "radar_level2_fallback_total",
            &[("reason", "low_sweep_ineligible"), ("stage", "poll")]
        ) >= 1.0
    );
    assert!(metrics::hist_count("radar_level2_permit_wait_seconds") > started);
    assert_eq!(pool.usage().await?.decodes_running, 0);
    assert_eq!(
        count(&pool, "SELECT count(*) FROM {s}.stations WHERE busy").await?,
        0
    );
    // A station nobody demands is skipped without touching NOAA.
    let skipped = poller.poll("KDTX").await?;
    assert_eq!(skipped.status, "skipped");
    Ok(())
}

#[tokio::test]
async fn saturated_decodes_skip_the_poll_instead_of_queueing() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut pool = replica(&url, &schema).await?;
    assert!(pool.station("KGRR").await?);
    pool.release_station("KGRR").await?;
    let (a, b) = (pool.decode().await?.unwrap(), pool.decode().await?.unwrap());
    let outcome = poller(&url, &schema, Arc::new(InMemory::new()))?
        .poll("KGRR")
        .await?;
    assert_eq!(
        (outcome.status, outcome.reason.as_deref()),
        ("skipped", Some("decode_saturated"))
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM {s}.stations WHERE busy").await?,
        0
    );
    pool.release_decode(&a).await?;
    pool.release_decode(&b).await?;
    Ok(())
}

#[tokio::test]
async fn reaper_frees_dead_worker_leases_and_cache_reservations_but_not_live_ones() -> Result<()> {
    let Some(url) = database() else { return Ok(()) };
    let schema = schema();
    let mut pool = replica(&url, &schema).await?;
    pool.beat("alive").await?;
    let render = pool.render_permit().await?.expect("render permit");
    let one = radar_level2::leases::Reservation::Granted;
    assert_eq!(
        pool.reserve("render", "tile:dead:1", 1024, 1 << 29).await?,
        one
    );
    assert_eq!(
        pool.reserve("render", "tile:alive:1", 1024, 1 << 29)
            .await?,
        one
    );
    pool.client
        .execute(
            &format!(
                "UPDATE {schema}.reservations SET created_at=clock_timestamp()-interval '5 minutes'"
            ),
            &[],
        )
        .await?;
    let reaped = pool.reap().await?;
    assert_eq!((reaped.reservations, reaped.renders), (1, 0));
    assert_eq!(
        count(
            &pool,
            "SELECT count(*) FROM {s}.reservations WHERE id='tile:alive:1'"
        )
        .await?,
        1
    );
    pool.client
        .execute(
            &format!("UPDATE {schema}.renders SET leased_at=clock_timestamp()-interval '16 minutes' WHERE owner=$1"),
            &[&render],
        )
        .await?;
    assert_eq!(pool.reap().await?.renders, 1);
    Ok(())
}
