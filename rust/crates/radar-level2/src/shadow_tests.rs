use super::*;
use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use std::sync::Mutex;

#[derive(Default)]
struct VecSink {
    seen: Mutex<Vec<SeenSweep>>,
    events: Mutex<Vec<Event>>,
    states: Mutex<Vec<(String, String, f64)>>,
}

#[async_trait]
impl Sink for VecSink {
    async fn seen(&self, row: &SeenSweep) -> Result<()> {
        self.seen.lock().unwrap().push(row.clone());
        Ok(())
    }
    async fn event(&self, e: &Event) -> Result<()> {
        self.events.lock().unwrap().push(e.clone());
        Ok(())
    }
    async fn state(&self, _: NaiveDate, station: &str, state: &str, seconds: f64) -> Result<()> {
        self.states
            .lock()
            .unwrap()
            .push((station.into(), state.into(), seconds));
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Fake {
    submits: Arc<Mutex<u32>>,
}

fn envelope(n: u32) -> Value {
    let end = format!("2026-11-16T12:{:02}:00Z", n.min(59));
    json!({
        "view_id": "v1",
        "revision": format!("r{n}"),
        "issued_at": format!("2026-11-16T12:{:02}:30Z", n.min(59)),
        "required_tiles": [{"z": 9, "x": 1, "y": 2}],
        "tile_url_template": crate::viewport::TILE_TEMPLATE,
        "manifest": {"layers": {"level2-ref-KGRR": {"frames": [
            {"path": format!("sweep{n}/t1"), "scan_end": end}
        ]}}},
    })
}

async fn fake_api() -> (String, Fake, tokio::task::JoinHandle<()>) {
    let fake = Fake::default();
    let app = Router::new()
        .route(
            "/api/level2/sites",
            get(|| async {
                Json(json!({"sites": [{"id": "KGRR", "lat": 42.9, "lon": -85.5, "enabled": true}]}))
            }),
        )
        .route(
            "/api/level2/views",
            post(|State(f): State<Fake>| async move {
                let mut n = f.submits.lock().unwrap();
                *n += 1;
                Json(json!({"state": "ready", "view_id": "v1", "manifest_url": format!("/m/{n}")}))
            }),
        )
        .route(
            "/m/{n}",
            get(
                |axum::extract::Path(n): axum::extract::Path<u32>| async move { Json(envelope(n)) },
            ),
        )
        .route(
            "/level2/tiles/{*rest}",
            get(|| async { ([("x-cache", "HIT")], b"\x89PNG....".to_vec()) }),
        )
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), fake, task)
}

fn cfg(api: &str) -> Config {
    Config {
        stations: vec!["KGRR".into()],
        api: api.into(),
        edge: api.into(),
        zoom: 9,
        palette: "classic".into(),
        cold_every: None,
        gap_s: GAP_S_DEFAULT,
        prom: None,
        prom_namespace: "radar-ng".into(),
        prom_pods: ".*".into(),
    }
}

#[test]
fn station_list_is_explicit() {
    assert!(stations_from("").is_err());
    assert!(stations_from(" , ").is_err());
    assert!(stations_from("KGRR,bad!").is_err());
    assert_eq!(stations_from(" kgrr, KDTX ").unwrap(), ["KGRR", "KDTX"]);
}

#[tokio::test]
async fn recorder_skips_first_frame_then_records_each_new_sweep() {
    let (api, _fake, task) = fake_api().await;
    let sink = Arc::new(VecSink::default());
    let mut c = cfg(&api);
    c.stations = vec!["KGRR".into()];
    let rec = Recorder::new(c, sink.clone()).unwrap();
    let site = rec.sites().await.unwrap().remove(0);
    let (mut seen, mut last) = (vec![], String::new());
    for _ in 0..3 {
        let (state, _) = rec.observe(&site, &mut seen, &mut last).await;
        assert_eq!(state, "ready");
    }
    let rows = sink.seen.lock().unwrap().clone();
    assert_eq!(rows.len(), 2, "first frame predates the recorder");
    assert_eq!(rows[0].sweep_id, "sweep2");
    assert_eq!(rows[0].revision, "r2");
    assert_eq!(rows[0].tile_status, 200);
    assert_eq!(rows[0].tile_cache.as_deref(), Some("HIT"));
    assert!(rows[0].tile_cached_ms.is_some());
    assert_eq!(
        rows[0].manifest_ready_at.to_rfc3339(),
        "2026-11-16T12:02:30+00:00"
    );
    task.abort();
}

#[tokio::test]
async fn disabled_station_is_refused() {
    let app = Router::new().route(
        "/api/level2/sites",
        get(|| async {
            Json(json!({"sites": [{"id": "KGRR", "lat": 1.0, "lon": 1.0, "enabled": false}]}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let rec = Recorder::new(cfg(&format!("http://{addr}")), Arc::new(VecSink::default())).unwrap();
    let err = rec.sites().await.err().unwrap().to_string();
    assert!(err.contains("not enabled"), "{err}");
    task.abort();
}

#[test]
fn percentile_is_nearest_rank() {
    let mut v: Vec<f64> = (1..=100).map(f64::from).collect();
    assert_eq!(percentile(&mut v, 95.0), Some(95.0));
    assert_eq!(percentile(&mut v, 50.0), Some(50.0));
    assert_eq!(percentile(&mut [], 50.0), None);
    assert_eq!(percentile(&mut [7.0], 99.0), Some(7.0));
}

fn t(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

fn clock(end: &str, arrive: &str, avail: &str, id: &str) -> SweepClock {
    SweepClock {
        station: "KGRR".into(),
        sweep_id: id.into(),
        volume_start: t(end),
        scan_start: t(end),
        scan_end: t(end),
        vcp: Some(215),
        elevation_number: 1,
        source_kind: "live".into(),
        source_arrival_at: Some(t(arrive)),
        first_available_at: t(avail),
        qc_ms: 10.0,
        rays: 720,
        missing_sectors: 0,
        missing_deg: 0.0,
        source_bytes: 1,
        artifact_bytes: 2,
        poll_ms: 3.0,
    }
}

#[test]
fn report_counts_valid_days_only_and_joins_clocks() {
    let d1: NaiveDate = "2026-11-01".parse().unwrap();
    let d2: NaiveDate = "2026-11-02".parse().unwrap();
    let d3: NaiveDate = "2026-11-03".parse().unwrap();
    let stations = vec!["KGRR".to_string()];
    let mut i = Inputs::default();
    i.states.push((d1, "KGRR".into(), "ready".into(), 86_000.0));
    i.states.push((d2, "KGRR".into(), "ready".into(), 1_000.0));
    i.clocks.push(clock(
        "2026-11-01T10:00:00Z",
        "2026-11-01T10:00:20Z",
        "2026-11-01T10:00:25Z",
        "a",
    ));
    i.clocks.push(clock(
        "2026-11-01T10:30:00Z",
        "2026-11-01T10:30:10Z",
        "2026-11-01T10:30:12Z",
        "b",
    ));
    i.clocks.push(clock(
        "2026-11-02T10:00:00Z",
        "2026-11-02T10:09:00Z",
        "2026-11-02T10:10:00Z",
        "c",
    ));
    i.seen.push(SeenSweep {
        station: "KGRR".into(),
        sweep_id: "a".into(),
        scan_end: t("2026-11-01T10:00:00Z"),
        view_id: "v".into(),
        revision: "r".into(),
        manifest_ready_at: t("2026-11-01T10:00:30Z"),
        manifest_ms: 40.0,
        edge_display_at: t("2026-11-01T10:00:40Z"),
        tile_cold_ms: 300.0,
        tile_cached_ms: Some(20.0),
        tile_status: 200,
        tile_cache: None,
    });
    let r = build_report(&i, &stations, d1, d3, GAP_S_DEFAULT);
    assert_eq!(r["valid_days"], 1, "day two has 1000 s of uptime");
    assert_eq!(r["sweeps"]["indexed"], 2);
    assert_eq!(r["sweeps"]["displayed"], 1);
    assert_eq!(r["gaps"]["undisplayed_sweeps"], 1);
    assert_eq!(r["gaps"]["feed_gaps"], 1, "30 min between a and b");
    assert_eq!(r["clocks_s"]["source_lag"]["max"], 20.0);
    assert_eq!(r["clocks_s"]["first_availability"]["p50"], 12.0);
    assert_eq!(r["clocks_s"]["manifest_readiness"]["max"], 5.0);
    assert_eq!(r["clocks_s"]["scan_end_to_edge_display"]["max"], 40.0);
    assert_eq!(r["gates"]["qc_ms_per_lowest_sweep"]["result"], "pass");
    assert_eq!(r["gates"]["manifest_ms_edge"]["result"], "pass");
    i.excluded.insert(d1, "drill".into());
    let r = build_report(&i, &stations, d1, d3, GAP_S_DEFAULT);
    assert_eq!(r["valid_days"], 0);
    assert_eq!(r["gates"]["qc_ms_per_lowest_sweep"]["result"], "no_data");
    assert!(render_report(&r).contains("valid days: 0"));
}

#[test]
fn tile_of_matches_viewport_math() {
    let [x, y] = tile_of(42.9, -85.5, 9);
    assert_eq!([x, y], [134, 188]);
}
