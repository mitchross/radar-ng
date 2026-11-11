use super::*;
use crate::{
    artifact::Artifact,
    decode::{Moment, Ray, Sweep, VolumeSite},
};
use futures_util::StreamExt;
use object_store::memory::InMemory;

struct Harness {
    url: String,
    schema: String,
    store: Arc<InMemory>,
    key: String,
    svc: Arc<TileService>,
}

fn dense_sweep() -> Sweep {
    let now = chrono::Utc::now();
    let rays = (0..720u16)
        .map(|n| Ray {
            number: n + 1,
            time: now,
            azimuth_deg: f32::from(n) * 0.5,
            elevation_deg: 0.5,
            azimuth_spacing_deg: 0.5,
            radial_status: match n {
                0 => 0,
                719 => 2,
                _ => 1,
            },
            blanking: 0,
            moments: [
                Some(Moment {
                    first_gate_m: 2125,
                    spacing_m: 250,
                    word_bits: 8,
                    scale: 2.0,
                    offset: 66.0,
                    control_flags: 0,
                    snr_threshold_raw: 0,
                    tover_raw: 0,
                    encoded: vec![100; 300],
                    values: (0..300)
                        .map(|g| ((n as usize + g) % 60) as f32 + 5.0)
                        .collect(),
                    qc: vec![0; 300],
                }),
                None,
                None,
            ],
        })
        .collect();
    Sweep {
        station: "KGRR".into(),
        volume_start: now,
        volume_number: "001".into(),
        ordinal: 1,
        elevation_number: 1,
        vcp: Some(35),
        site: Some(VolumeSite {
            latitude_deg: 42.89389,
            longitude_deg: -85.54489,
            site_height_m_raw: 0,
            tower_height_m_raw: 0,
        }),
        scan_start: now,
        scan_end: now,
        rays,
        missing_sectors_deg: vec![],
    }
}

async fn pool(url: &str, schema: &str) -> Pool {
    let pool = Pool::connect(url, schema).await.unwrap();
    pool.initialize().await.unwrap();
    pool
}

async fn harness(limits: Limits) -> Option<Harness> {
    let Ok(url) = std::env::var("LEVEL2_TEST_PG") else {
        eprintln!("SKIP: LEVEL2_TEST_PG absent");
        return None;
    };
    let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
    let store = Arc::new(InMemory::new());
    let artifact = Artifact::build(&dense_sweep(), vec![], "level2/test").unwrap();
    artifact.publish(store.as_ref()).await.unwrap();
    let svc = TileService::new(pool(&url, &schema).await, store.clone(), limits).unwrap();
    Some(Harness {
        url,
        schema,
        store,
        key: artifact.commit_key,
        svc,
    })
}

impl Harness {
    fn req(&self, dx: u32, dy: u32) -> TileRequest {
        let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
        TileRequest {
            commit_key: self.key.clone(),
            palette: "classic".into(),
            z: 9,
            x: x + dx,
            y: y + dy,
        }
    }
    async fn count(&self, sql: &str) -> i64 {
        let pool = self.svc.pool.lock().await;
        pool.client
            .query_one(&sql.replace("{s}", &self.schema), &[])
            .await
            .unwrap()
            .get(0)
    }
    async fn permits(&self) -> i64 {
        self.count("SELECT count(*) FROM {s}.renders WHERE owner IS NOT NULL")
            .await
    }
    async fn reserved(&self, kind: &str) -> i64 {
        self.count(&format!(
            "SELECT COALESCE(sum(bytes),0)::bigint FROM {{s}}.reservations WHERE kind='{kind}'"
        ))
        .await
    }
    async fn tile_objects(&self) -> usize {
        self.store
            .list(Some(&Path::from("level2/test/tiles")))
            .count()
            .await
    }
    async fn drained(&self) {
        for _ in 0..200 {
            if self.permits().await == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("permits never released");
    }
}

fn close(svc: &TileService) -> Arc<(Mutex<bool>, std::sync::Condvar)> {
    let latch = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    *svc.latch.lock().unwrap() = Some(latch.clone());
    latch
}

fn open(latch: &(Mutex<bool>, std::sync::Condvar)) {
    *latch.0.lock().unwrap() = true;
    latch.1.notify_all();
}

fn ready(o: Outcome) -> Ready {
    match o {
        Outcome::Ready(r) => r,
        Outcome::Fallback(why) => panic!("fallback {why}"),
    }
}

fn why(o: &Outcome) -> &'static str {
    match o {
        Outcome::Fallback(why) => why,
        Outcome::Ready(_) => "ready",
    }
}

#[tokio::test]
async fn render_publishes_one_verified_immutable_tile_and_accounts_memory() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let r = ready(h.svc.render(h.req(0, 0), Duration::from_secs(30)).await);
    assert_eq!(r.source, "rendered");
    assert!(!r.transparent && r.png.starts_with(&radar_tiler::png::PNG_SIGNATURE));
    let stored = h
        .store
        .get(&Path::from(r.key.as_str()))
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(crate::checksum(&stored), r.sha256);
    assert_eq!(stored.len(), r.png.len());
    assert!(
        write_immutable(h.store.as_ref(), &r.key, b"different")
            .await
            .is_err()
    );
    assert_eq!(h.reserved("render").await, r.png.len() as i64);
    let sweep_bytes = h
        .svc
        .state
        .lock()
        .unwrap()
        .sweeps
        .values()
        .next()
        .unwrap()
        .value
        .bytes();
    assert_eq!(h.reserved("sweep").await, sweep_bytes as i64);
    assert!(h.svc.peak_scratch_bytes() + r.png.len() <= TILE_SCRATCH_BYTES);
    assert_eq!(h.permits().await, 0);
    assert_eq!(
        ready(h.svc.render(h.req(0, 0), Duration::from_secs(5)).await).source,
        "hot_cache"
    );
    let other = TileService::new(
        pool(&h.url, &h.schema).await,
        h.store.clone(),
        Limits::default(),
    )
    .unwrap();
    let again = ready(other.render(h.req(0, 0), Duration::from_secs(5)).await);
    assert_eq!(
        (again.source, again.sha256.as_str()),
        ("rustfs", r.sha256.as_str())
    );
    assert_eq!(other.jobs_started(), 0);
    println!(
        "service tile: png_bytes={} peak_scratch_bytes={} sweep_bytes={sweep_bytes}",
        r.png.len(),
        h.svc.peak_scratch_bytes()
    );
}

#[tokio::test]
async fn stampede_on_one_tile_renders_once() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let latch = close(&h.svc);
    let calls: Vec<_> = (0..32)
        .map(|_| {
            let s = h.svc.clone();
            let r = h.req(0, 0);
            tokio::spawn(async move { s.render(r, Duration::from_secs(30)).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(300)).await;
    open(&latch);
    let shas: std::collections::BTreeSet<_> = futures_util::future::join_all(calls)
        .await
        .into_iter()
        .map(|o| ready(o.unwrap()).sha256)
        .collect();
    assert_eq!(
        (shas.len(), h.svc.jobs_started(), h.tile_objects().await),
        (1, 1, 1)
    );
}

#[tokio::test]
async fn burst_of_distinct_tiles_overflows_to_mrms_instead_of_queueing() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let latch = close(&h.svc);
    let start = std::time::Instant::now();
    let calls: Vec<_> = (0..6)
        .map(|i| {
            let s = h.svc.clone();
            let r = h.req(i % 3, i / 3);
            tokio::spawn(async move { s.render(r, Duration::from_secs(30)).await })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(h.permits().await, 2);
    assert_eq!(h.svc.jobs_started(), 2);
    let finished: Vec<_> = calls
        .iter()
        .filter(|c| c.is_finished())
        .count()
        .to_string()
        .into_bytes();
    assert_eq!(finished, b"4");
    assert!(start.elapsed() < Duration::from_secs(5));
    open(&latch);
    let outcomes: Vec<_> = futures_util::future::join_all(calls)
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| why(o) == "render_saturated")
            .count(),
        4
    );
    assert_eq!(outcomes.iter().filter(|o| why(o) == "ready").count(), 2);
    h.drained().await;
}

#[tokio::test]
async fn timed_out_jobs_keep_their_permits_until_blocking_work_ends() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let latch = close(&h.svc);
    for i in 0..2 {
        assert_eq!(
            why(&h.svc.render(h.req(i, 0), Duration::from_millis(300)).await),
            "render_timeout"
        );
    }
    assert_eq!((h.permits().await, h.svc.jobs_started()), (2, 2));
    assert_eq!(
        why(&h.svc.render(h.req(2, 0), Duration::from_secs(5)).await),
        "render_saturated"
    );
    assert!(h.reserved("render").await >= 2 * TILE_SCRATCH_BYTES as i64);
    open(&latch);
    h.drained().await;
    assert_eq!((h.tile_objects().await, h.reserved("render").await), (0, 0));
    assert_eq!(
        why(&h.svc.render(h.req(2, 0), Duration::from_secs(30)).await),
        "ready"
    );
}

#[tokio::test]
async fn dropped_requests_cancel_the_job_and_free_everything() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let latch = close(&h.svc);
    let (svc, req) = (h.svc.clone(), h.req(0, 0));
    let call = tokio::spawn(async move { svc.render(req, Duration::from_secs(60)).await });
    while h.svc.jobs_started() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    call.abort();
    let _ = call.await;
    assert_eq!(h.permits().await, 1);
    open(&latch);
    h.drained().await;
    assert_eq!((h.tile_objects().await, h.reserved("render").await), (0, 0));
}

#[tokio::test]
async fn oversized_work_falls_back_without_leaking_permits() {
    let Some(h) = harness(Limits {
        render: TILE_SCRATCH_BYTES - 1,
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    assert_eq!(
        why(&h.svc.render(h.req(0, 0), Duration::from_secs(5)).await),
        "render_cache_full"
    );
    h.drained().await;
    let Some(h) = harness(Limits {
        sweep: 1024,
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    assert_eq!(
        why(&h.svc.render(h.req(0, 0), Duration::from_secs(5)).await),
        "sweep_cache_full"
    );
    h.drained().await;
    assert_eq!(
        (h.reserved("render").await, h.reserved("sweep").await),
        (0, 0)
    );
    let Some(h) = harness(Limits {
        scratch: 1000,
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    assert_eq!(
        why(&h.svc.render(h.req(0, 0), Duration::from_secs(5)).await),
        "render_oversized"
    );
    assert_eq!(h.tile_objects().await, 0);
    for bad in [
        TileRequest {
            z: 7,
            ..h.req(0, 0)
        },
        TileRequest {
            palette: "x".into(),
            ..h.req(0, 0)
        },
        TileRequest {
            x: 1 << 9,
            ..h.req(0, 0)
        },
    ] {
        assert_eq!(
            why(&h.svc.render(bad, Duration::from_secs(1)).await),
            "invalid_tile"
        );
    }
}

#[tokio::test]
async fn pool_wide_caps_hold_across_services_and_decodes() {
    let Some(h) = harness(Limits::default()).await else {
        return;
    };
    let commit = artifact::read_commit(h.store.as_ref(), &h.key)
        .await
        .unwrap();
    let cap = 3 * artifact::wanted_bytes(&commit) + MIB;
    let (a, b) = (
        TileService::new(
            pool(&h.url, &h.schema).await,
            h.store.clone(),
            Limits {
                sweep: cap,
                ..Limits::default()
            },
        )
        .unwrap(),
        TileService::new(
            pool(&h.url, &h.schema).await,
            h.store.clone(),
            Limits {
                sweep: cap,
                ..Limits::default()
            },
        )
        .unwrap(),
    );
    assert_eq!(
        why(&a.render(h.req(0, 0), Duration::from_secs(30)).await),
        "ready"
    );
    assert_eq!(
        why(&b.render(h.req(1, 0), Duration::from_secs(30)).await),
        "sweep_cache_full"
    );
    let mut direct = pool(&h.url, &h.schema).await;
    let room = (HOT_LIMIT - crate::JOB_RESERVATION) as i64;
    let used = h.reserved("sweep").await;
    assert_eq!(
        direct
            .reserve("sweep", "fill", room - used + 1, HOT_LIMIT as i64)
            .await
            .unwrap(),
        Reservation::Granted
    );
    assert!(direct.decode().await.unwrap().is_none());
    assert_eq!(
        direct
            .reserve("sweep", "fill", 1, HOT_LIMIT as i64)
            .await
            .unwrap(),
        Reservation::Duplicate
    );
    direct.release("fill").await.unwrap();
    let one = direct.decode().await.unwrap().unwrap();
    assert!(
        direct.decode().await.unwrap().is_none(),
        "a resident sweep and two decodes exceed 1 GiB"
    );
    assert_eq!(
        direct
            .reserve("sweep", "late", room - used + 1, HOT_LIMIT as i64)
            .await
            .unwrap(),
        Reservation::Full
    );
    direct.release_decode(&one).await.unwrap();
    a.evict_sweep().map(|id| id.len()).expect("idle sweep");
}

#[tokio::test]
async fn rendered_cache_evicts_its_own_entries_under_the_pool_cap() {
    let Some(h) = harness(Limits {
        render: TILE_SCRATCH_BYTES + 12_000,
        ..Limits::default()
    })
    .await
    else {
        return;
    };
    for i in 0..6 {
        let t = std::time::Instant::now();
        let o = h
            .svc
            .render(h.req(i % 3, i / 3), Duration::from_secs(30))
            .await;
        println!("tile {i} {:?} {}", t.elapsed(), why(&o));
        let r = ready(o);
        assert!(!r.png.is_empty());
        assert!(h.reserved("render").await <= (TILE_SCRATCH_BYTES + 12_000) as i64);
    }
    assert!(h.svc.state.lock().unwrap().tiles.len() < 6);
    assert_eq!(h.tile_objects().await, 6);
}
