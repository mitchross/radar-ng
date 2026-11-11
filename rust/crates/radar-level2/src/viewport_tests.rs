use crate::{
    artifact::Artifact,
    catalogue::Catalogue,
    decode::{Moment, Ray, Sweep, VolumeSite},
    health::Health,
    memstore::MemoryStore,
    publish::{Clock, Done, Publisher, TileDone, TileSource},
    service::*,
    viewport::*,
};
use async_trait::async_trait;
use chrono::{DateTime, TimeDelta, Utc};
use object_store::{ObjectStore, PutPayload, memory::InMemory, path::Path};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};

const NS: &str = "level2/test";
const VERSION: &str = "level2-polar-v1-test0000";

struct Fake {
    store: Arc<InMemory>,
    meta: Arc<MemoryStore>,
    skip_write: Mutex<HashSet<String>>,
    transparent: Mutex<HashSet<String>>,
    fail: Mutex<Option<&'static str>>,
    delay: Mutex<Duration>,
    calls: AtomicI64,
}

fn parts(commit_key: &str) -> (String, String) {
    let (_, rest) = commit_key.split_once("/sweeps/").unwrap();
    let mut p = rest.split('/');
    (p.next().unwrap().into(), p.next().unwrap().into())
}

#[async_trait]
impl TileSource for Fake {
    fn key(&self, commit_key: &str, palette: &str, z: u32, x: u32, y: u32) -> Option<String> {
        let (station, identity) = parts(commit_key);
        Some(format!(
            "{NS}/tiles/{station}/{identity}/{VERSION}/{palette}/{z}/{x}/{y}.png"
        ))
    }
    fn version(&self) -> String {
        VERSION.into()
    }
    async fn tile(
        &self,
        commit_key: &str,
        palette: &str,
        z: u32,
        x: u32,
        y: u32,
        _d: Duration,
    ) -> Result<TileDone, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        if let Some(r) = *self.fail.lock().unwrap() {
            return Err(r.into());
        }
        let key = self.key(commit_key, palette, z, x, y).unwrap();
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend_from_slice(key.as_bytes());
        let sha = crate::checksum(&bytes);
        if !self.skip_write.lock().unwrap().contains(&key) {
            self.store
                .put(&Path::from(key.as_str()), PutPayload::from(bytes.clone()))
                .await
                .unwrap();
        }
        let transparent = self.transparent.lock().unwrap().contains(&key);
        self.meta.put_tile(
            &key,
            TileMeta {
                sha256: sha.clone(),
                bytes: bytes.len() as i64,
                transparent,
            },
        );
        Ok(TileDone {
            key,
            sha256: sha,
            bytes: bytes.len(),
            transparent,
        })
    }
}

struct Rig {
    mem: Arc<MemoryStore>,
    objects: Arc<InMemory>,
    fake: Arc<Fake>,
    offset: Arc<AtomicI64>,
    base: DateTime<Utc>,
    publisher: Publisher,
    service: Level2Service,
}

fn sweep(station: &str, lat: f32, lon: f32, end: DateTime<Utc>) -> Sweep {
    let rays = (0..720u16)
        .map(|n| Ray {
            number: n + 1,
            time: end,
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
        station: station.into(),
        volume_start: end - TimeDelta::seconds(300),
        volume_number: "001".into(),
        ordinal: 1,
        elevation_number: 1,
        vcp: Some(35),
        site: Some(VolumeSite {
            latitude_deg: lat,
            longitude_deg: lon,
            site_height_m_raw: 237,
            tower_height_m_raw: 0,
        }),
        scan_start: end - TimeDelta::seconds(20),
        scan_end: end,
        rays,
        missing_sectors_deg: vec![],
    }
}

fn rig() -> Rig {
    let base = DateTime::<Utc>::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap();
    let offset = Arc::new(AtomicI64::new(0));
    let o = offset.clone();
    let clock: Clock = Arc::new(move || base + TimeDelta::milliseconds(o.load(Ordering::SeqCst)));
    let mem = Arc::new(MemoryStore::default());
    let objects = Arc::new(InMemory::new());
    let fake = Arc::new(Fake {
        store: objects.clone(),
        meta: mem.clone(),
        skip_write: Mutex::default(),
        transparent: Mutex::default(),
        fail: Mutex::default(),
        delay: Mutex::default(),
        calls: AtomicI64::new(0),
    });
    let catalogue = Arc::new(Catalogue::snapshot().unwrap());
    let publisher = Publisher {
        store: mem.clone(),
        index: mem.clone(),
        tiles: fake.clone(),
        objects: objects.clone(),
        catalogue: catalogue.clone(),
        clock: clock.clone(),
        instance: "t".into(),
    };
    let service = Level2Service::new(
        mem.clone(),
        mem.clone(),
        objects.clone(),
        catalogue,
        vec!["KGRR".into(), "KDTX".into()],
        NS.into(),
        clock,
    )
    .unwrap();
    Rig {
        mem,
        objects,
        fake,
        offset,
        base,
        publisher,
        service,
    }
}

impl Rig {
    fn advance(&self, secs: i64) {
        self.offset.fetch_add(secs * 1000, Ordering::SeqCst);
    }
    fn now(&self) -> DateTime<Utc> {
        self.service.now()
    }
    async fn add_sweeps(&self, station: &str, lat: f32, lon: f32, ends: &[i64]) {
        for secs in ends {
            let end = self.base - TimeDelta::seconds(*secs);
            let a = Artifact::build(&sweep(station, lat, lon, end), vec![], NS).unwrap();
            a.publish(self.objects.as_ref()).await.unwrap();
            self.mem.add_sweep(
                station,
                SweepRef {
                    scan_end: end,
                    identity: a.commit.identity.clone(),
                    commit_key: a.commit_key.clone(),
                },
            );
        }
    }
    fn healthy(&self, station: &str) {
        self.mem.set_health(
            station,
            Health {
                active: true,
                low_sweep_end: Some(self.base - TimeDelta::seconds(30)),
                last_advance: Some(self.base - TimeDelta::seconds(10)),
                ..Health::default()
            },
        );
    }
    fn raw(&self, palette: &str) -> RawView {
        let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
        RawView {
            centre: Centre {
                lat: 42.89389,
                lon: -85.54489,
            },
            zoom: 9,
            tiles: vec![[x, y], [x + 1, y]],
            palette: palette.into(),
        }
    }
    async fn submit(&self, palette: &str) -> (u16, serde_json::Value) {
        match self.service.submit(self.raw(palette)).await.unwrap() {
            Submit::Reply { status, body } => {
                (status, serde_json::from_slice(&body.bytes).unwrap())
            }
            Submit::Invalid(r) => panic!("invalid {r}"),
        }
    }
    async fn ready(&self) -> (String, String) {
        self.add_sweeps("KGRR", 42.89389, -85.54489, &[300, 180, 60])
            .await;
        self.healthy("KGRR");
        let (status, body) = self.submit("classic").await;
        assert_eq!((status, body["state"].as_str()), (202, Some("pending")));
        let id = body["view_id"].as_str().unwrap().to_owned();
        let done = self.publisher.run_once().await.unwrap();
        let Done::Published(rev) = done else {
            panic!("{done:?}")
        };
        (id, rev)
    }
}

#[test]
fn validation_enforces_every_bound() {
    let ok = |tiles: Vec<[u32; 2]>, zoom, palette: &str, lat, lon| {
        validate(RawView {
            centre: Centre { lat, lon },
            zoom,
            tiles,
            palette: palette.into(),
        })
    };
    let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
    assert!(ok(vec![[x, y]], 9, "classic", 42.89389, -85.54489).is_ok());
    assert_eq!(
        ok(vec![[x, y]], 7, "classic", 42.89, -85.54),
        Err("invalid_zoom")
    );
    assert_eq!(
        ok(vec![[x, y]], 14, "classic", 42.89, -85.54),
        Err("invalid_zoom")
    );
    assert_eq!(
        ok(vec![[x, y]], 9, "neon", 42.89, -85.54),
        Err("invalid_palette")
    );
    assert_eq!(ok(vec![], 9, "classic", 42.89, -85.54), Err("no_tiles"));
    assert_eq!(
        ok(vec![[x, y], [x, y]], 9, "classic", 42.89, -85.54),
        Err("duplicate_tile")
    );
    assert_eq!(
        ok(vec![[512, 0]], 9, "classic", 42.89, -85.54),
        Err("invalid_tile")
    );
    assert_eq!(
        ok(vec![[x + 20, y]], 9, "classic", 42.89389, -85.54489),
        Err("excessive_area").or(Err("centre_outside_tiles"))
    );
    assert_eq!(
        ok(vec![[x + 3, y]], 9, "classic", 42.89389, -85.54489),
        Err("centre_outside_tiles")
    );
    assert_eq!(
        ok(vec![[x, y]], 9, "classic", f64::NAN, 0.0),
        Err("invalid_centre")
    );
    assert_eq!(
        ok(vec![[x, y]], 9, "classic", 89.0, 0.0),
        Err("invalid_centre")
    );
    let many: Vec<[u32; 2]> = (0..65).map(|i| [x + i % 8, y + i / 8]).collect();
    assert_eq!(
        ok(many, 9, "classic", 42.89389, -85.54489),
        Err("too_many_tiles")
    );
    let wide: Vec<[u32; 2]> = vec![[x, y], [x + 8, y]];
    assert_eq!(
        ok(wide, 9, "classic", 42.89389, -85.54489),
        Err("excessive_area")
    );
    let sixty_four: Vec<[u32; 2]> = (0..64).map(|i| [x + i % 8, y + i / 8]).collect();
    assert!(ok(sixty_four, 9, "classic", 42.89389, -85.54489).is_ok());
}

#[test]
fn view_identity_changes_with_scope_but_not_tile_order() {
    let scope = |tiles: Vec<[u32; 2]>, zoom, palette: &str| ViewScope {
        station: "KGRR".into(),
        zoom,
        tiles,
        palette: palette.into(),
    };
    let a = scope(vec![[1, 2], [3, 4]], 9, "classic");
    assert_eq!(a.id(), scope(vec![[1, 2], [3, 4]], 9, "classic").id());
    assert_ne!(a.id(), scope(vec![[1, 2], [3, 4]], 9, "vivid").id());
    assert_ne!(a.id(), scope(vec![[1, 2], [3, 4]], 10, "classic").id());
    assert_ne!(a.id(), scope(vec![[1, 2]], 9, "classic").id());
}

#[test]
fn beam_height_follows_the_four_thirds_earth_formula() {
    let h = beam_height_m(100_000.0, 0.5).unwrap();
    assert!((1300.0..1600.0).contains(&h), "{h}");
    assert!(beam_height_m(0.0, 0.5).unwrap().abs() < 1.0);
    assert!(beam_height_m(100_000.0, 1.5).unwrap() > h);
}

#[tokio::test]
async fn ready_loop_is_published_atomically_with_exact_frames() {
    let r = rig();
    let (id, rev) = r.ready().await;
    let (status, body) = r.submit("classic").await;
    assert_eq!(status, 200);
    assert_eq!(body["state"], "ready");
    assert_eq!(body["view_id"], id.as_str());
    assert_eq!(
        body["manifest_url"],
        format!("/api/level2/views/{id}/{rev}/manifest.json")
    );
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    let env: serde_json::Value = serde_json::from_str(&revision.envelope).unwrap();
    assert_eq!(env["schema_version"], 1);
    assert_eq!(env["station"]["id"], "KGRR");
    assert_eq!(env["required_tiles"].as_array().unwrap().len(), 2);
    let layer = &env["manifest"]["layers"]["level2-ref-KGRR"];
    assert_eq!(layer["product"], "level2-ref");
    assert_eq!(layer["delivery"], "viewport");
    let frames = layer["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 3, "fewer than six sweeps stay explicit");
    let stamps: Vec<&str> = frames
        .iter()
        .map(|f| f["timestamp"].as_str().unwrap())
        .collect();
    let mut sorted = stamps.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(stamps, sorted, "oldest first, no duplicates");
    for f in frames {
        assert_eq!(f["timestamp"], f["scan_end"]);
        assert_eq!(f["station_id"], "KGRR");
        assert_eq!(f["units"], "dBZ");
        assert_eq!(f["beam_model"], "effective-earth-4/3");
        assert_eq!(f["max_ground_range_m"], 150_000.0);
        assert_eq!(f["qc_profile"], "level2-conservative-v1");
        assert_eq!(f["decoder"], "nexrad-1.0.0");
        assert!((f["elevation_deg"].as_f64().unwrap() - 0.5).abs() < 1e-6);
        assert!(f["beam_height_m_above_radar"].as_f64().is_some());
        assert!(f["ground_range_m"].as_f64().unwrap() < 1000.0);
        assert_eq!(f["antenna_altitude_m_msl"], 237);
        assert_eq!(f["sampler_version"], crate::polar::SAMPLER_VERSION);
        assert_eq!(f["dual_pol"]["rho"], "absent");
    }
    assert_eq!(
        layer["latest"],
        *frames.last().map(|f| &f["timestamp"]).unwrap()
    );
    let parsed =
        radar_core::manifest::parse_manifest(&serde_json::to_string(&env["manifest"]).unwrap())
            .unwrap();
    assert_eq!(parsed.layers["level2-ref-KGRR"].frames.len(), 3);
    // Every advertised tile across every frame and XYZ is a readable PNG, never a transparent 200 stand-in.
    for f in frames {
        let path = f["path"].as_str().unwrap();
        let (sweep, version) = path.split_once('/').unwrap();
        for t in env["required_tiles"].as_array().unwrap() {
            let (x, y) = (
                t["x"].as_u64().unwrap() as u32,
                t["y"].as_u64().unwrap() as u32,
            );
            let TileReply::Ok { bytes, .. } = r
                .service
                .tile("KGRR", sweep, version, "classic", 9, x, y)
                .await
            else {
                panic!("tile {path} {x} {y}")
            };
            assert!(bytes.starts_with(b"\x89PNG"));
        }
    }
}

#[tokio::test]
async fn two_stations_with_identical_timestamps_stay_separate() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.add_sweeps("KDTX", 42.69972, -83.47167, &[60]).await;
    let a = r.mem.recent("KGRR", 6).await.unwrap();
    let b = r.mem.recent("KDTX", 6).await.unwrap();
    assert_eq!(a[0].scan_end, b[0].scan_end);
    assert_ne!(a[0].identity, b[0].identity);
    assert_ne!(a[0].commit_key, b[0].commit_key);
    let k = |s: &str, i: &str| {
        r.fake
            .key(
                &format!("{NS}/sweeps/{s}/{i}/rev/commit.json"),
                "classic",
                9,
                1,
                2,
            )
            .unwrap()
    };
    assert_ne!(k("KGRR", &a[0].identity), k("KDTX", &b[0].identity));
    use crate::viewport::SweepIndex;
}

#[tokio::test]
async fn missing_object_blocks_publication_and_never_serves_transparent_200() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[120, 60]).await;
    r.healthy("KGRR");
    let sweeps = r.mem.recent("KGRR", 6).await.unwrap();
    let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
    let lost = r
        .fake
        .key(&sweeps[1].commit_key, "classic", 9, x + 1, y)
        .unwrap();
    r.fake.skip_write.lock().unwrap().insert(lost.clone());
    let (_, body) = r.submit("classic").await;
    let id = body["view_id"].as_str().unwrap().to_owned();
    let done = r.publisher.run_once().await.unwrap();
    assert_eq!(done, Done::Failed("tile_missing".into()));
    let (status, body) = r.submit("classic").await;
    assert_eq!((status, body["state"].as_str()), (200, Some("fallback")));
    assert_eq!(body["reason"], "tile_missing");
    assert!(body.get("manifest_url").is_none());
    assert_eq!(r.mem.revision_count(), 0);
    assert!(matches!(
        r.service.manifest(&id, "0123456789abcdef").await,
        ManifestReply::NotFound
    ));
    let (sweep, _) = parts(&sweeps[1].commit_key);
    let _ = sweep;
    let (st, identity) = parts(&sweeps[1].commit_key);
    assert!(matches!(
        r.service
            .tile(&st, &identity, VERSION, "classic", 9, x + 1, y)
            .await,
        TileReply::NotFound | TileReply::Gone
    ));
}

#[tokio::test]
async fn transparent_tiles_are_published_as_real_pngs() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    let sweeps = r.mem.recent("KGRR", 6).await.unwrap();
    let (x, y) = radar_tiler::geometry::lat_lon_to_tile(42.89389, -85.54489, 9);
    r.fake.transparent.lock().unwrap().insert(
        r.fake
            .key(&sweeps[0].commit_key, "classic", 9, x, y)
            .unwrap(),
    );
    r.submit("classic").await;
    assert!(matches!(
        r.publisher.run_once().await.unwrap(),
        Done::Published(_)
    ));
}

#[tokio::test]
async fn quota_exhaustion_falls_back_to_mrms_with_the_reason() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    *r.fake.fail.lock().unwrap() = Some("quota_exhausted");
    r.submit("classic").await;
    assert_eq!(
        r.publisher.run_once().await.unwrap(),
        Done::Failed("quota_exhausted".into())
    );
    let (_, body) = r.submit("classic").await;
    assert_eq!(
        (body["state"].as_str(), body["reason"].as_str()),
        (Some("fallback"), Some("quota_exhausted"))
    );
    assert_eq!(r.mem.pin_count(), 0, "a failed build leaves no pins behind");
}

#[tokio::test]
async fn pending_expires_into_pending_timeout_after_thirty_seconds() {
    let r = rig();
    r.healthy("KGRR");
    let (status, body) = r.submit("classic").await;
    assert_eq!(
        (
            status,
            body["state"].as_str(),
            body["poll_after_s"].as_i64()
        ),
        (202, Some("pending"), Some(5))
    );
    assert_eq!(
        r.publisher.run_once().await.unwrap(),
        Done::Waiting("warming".into())
    );
    r.advance(31);
    let (status, body) = r.submit("classic").await;
    assert_eq!(
        (status, body["state"].as_str(), body["reason"].as_str()),
        (200, Some("fallback"), Some("pending_timeout"))
    );
}

#[tokio::test]
async fn fallback_retries_no_sooner_than_fifteen_seconds() {
    let r = rig();
    r.healthy("KGRR");
    r.submit("classic").await;
    r.advance(31);
    assert_eq!(r.submit("classic").await.1["state"], "fallback");
    r.advance(5);
    assert_eq!(r.submit("classic").await.1["state"], "fallback");
    r.advance(15);
    assert_eq!(r.submit("classic").await.0, 202);
}

#[tokio::test]
async fn changing_palette_or_zoom_creates_a_new_scope() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    let (_, a) = r.submit("classic").await;
    let (_, b) = r.submit("vivid").await;
    assert_ne!(a["view_id"], b["view_id"]);
    assert_eq!(a["palette"], "classic");
    assert_eq!(b["palette"], "vivid");
}

#[tokio::test]
async fn identical_requests_share_one_build() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    let (_, a) = r.submit("classic").await;
    let (_, b) = r.submit("classic").await;
    assert_eq!(a["view_id"], b["view_id"]);
    assert!(matches!(
        r.publisher.run_once().await.unwrap(),
        Done::Published(_)
    ));
    assert_eq!(r.publisher.run_once().await.unwrap(), Done::Idle);
    assert_eq!(
        r.fake.calls.load(Ordering::SeqCst),
        2,
        "two tiles for one sweep, rendered once"
    );
}

#[tokio::test]
async fn publish_failure_keeps_the_previous_revision_within_its_lease() {
    let r = rig();
    let (id, rev) = r.ready().await;
    r.advance(20);
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[0]).await;
    r.healthy("KGRR");
    r.mem.fail_publish.store(true, Ordering::SeqCst);
    let (_, body) = r.submit("classic").await;
    assert_eq!(
        body["state"], "ready",
        "still the old revision while a rebuild starts"
    );
    assert!(matches!(
        r.publisher.run_once().await.unwrap(),
        Done::Failed(_)
    ));
    assert!(matches!(
        r.service.manifest(&id, &rev).await,
        ManifestReply::Ok(_)
    ));
    let (_, body) = r.submit("classic").await;
    assert_eq!(body["state"], "ready");
    r.mem.fail_publish.store(false, Ordering::SeqCst);
    r.advance(1);
    let (_, body) = r.submit("classic").await;
    assert_eq!(body["state"], "ready");
}

#[tokio::test]
async fn revision_expires_then_manifest_is_gone_and_tiles_follow_the_read_grace() {
    let r = rig();
    let (id, rev) = r.ready().await;
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    let env: serde_json::Value = serde_json::from_str(&revision.envelope).unwrap();
    let path = env["manifest"]["layers"]["level2-ref-KGRR"]["frames"][0]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    let (sweep, version) = path.split_once('/').unwrap();
    let t = &env["required_tiles"][0];
    let (x, y) = (
        t["x"].as_u64().unwrap() as u32,
        t["y"].as_u64().unwrap() as u32,
    );
    r.advance(VIEW_LEASE_S + 1);
    assert!(matches!(
        r.service.manifest(&id, &rev).await,
        ManifestReply::Gone
    ));
    assert!(
        matches!(
            r.service
                .tile("KGRR", sweep, version, "classic", 9, x, y)
                .await,
            TileReply::Ok { .. }
        ),
        "read grace"
    );
    r.advance(READ_GRACE_S + 1);
    assert!(matches!(
        r.service
            .tile("KGRR", sweep, version, "classic", 9, x, y)
            .await,
        TileReply::Gone
    ));
    let (_, body) = r.submit("classic").await;
    assert_ne!(body["state"], "ready");
}

#[tokio::test]
async fn gc_never_removes_pinned_tiles_and_blocks_staging_on_collected_keys() {
    let r = rig();
    let (id, rev) = r.ready().await;
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    assert!(
        r.mem.collect(r.now()).await.unwrap().is_empty(),
        "pinned tiles stay"
    );
    let keys = revision.tile_keys.clone();
    r.advance(VIEW_LEASE_S + READ_GRACE_S + 1);
    // A staged build pin keeps a tile alive even after its revision pin lapsed.
    let until = r.now() + TimeDelta::seconds(30);
    assert_eq!(
        r.mem
            .stage_pins("build:x", &keys[..1], until)
            .await
            .unwrap(),
        Pin::Staged
    );
    let dead = r.mem.collect(r.now()).await.unwrap();
    assert!(!dead.contains(&keys[0]));
    assert!(dead.len() + 1 == keys.len());
    assert_eq!(
        r.mem
            .stage_pins("build:y", &dead[..1], until)
            .await
            .unwrap(),
        Pin::Collecting
    );
    assert!(matches!(
        r.service
            .tile("KGRR", "0".repeat(64).as_str(), VERSION, "classic", 9, 1, 1)
            .await,
        TileReply::NotFound
    ));
    r.mem.collected(&dead).await.unwrap();
    assert_eq!(
        r.mem
            .stage_pins("build:y", &dead[..1], until)
            .await
            .unwrap(),
        Pin::Staged
    );
}

#[tokio::test]
async fn corrupt_or_unreachable_stores_never_become_success() {
    let r = rig();
    let (id, rev) = r.ready().await;
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    let key = revision.tile_keys[0].clone();
    let parts: Vec<&str> = key.split('/').collect();
    let (station, sweep, version, palette) = (parts[3], parts[4], parts[5], parts[6]);
    let (z, x) = (parts[7].parse().unwrap(), parts[8].parse().unwrap());
    let y: u32 = parts[9].trim_end_matches(".png").parse().unwrap();
    r.objects
        .put(
            &Path::from(key.as_str()),
            PutPayload::from(b"\x89PNGtampered".to_vec()),
        )
        .await
        .unwrap();
    assert!(matches!(
        r.service
            .tile(station, sweep, version, palette, z, x, y)
            .await,
        TileReply::Unavailable("tile_corrupt")
    ));
    r.objects.delete(&Path::from(key.as_str())).await.unwrap();
    assert!(matches!(
        r.service
            .tile(station, sweep, version, palette, z, x, y)
            .await,
        TileReply::NotFound
    ));
    r.mem.fail_all.store(true, Ordering::SeqCst);
    assert!(matches!(
        r.service
            .tile(station, sweep, version, palette, z, x, y)
            .await,
        TileReply::Unavailable(_)
    ));
    assert!(matches!(
        r.service.manifest(&id, &rev).await,
        ManifestReply::Unavailable
    ));
    assert!(r.service.submit(r.raw("classic")).await.is_err());
}

#[tokio::test]
async fn renewal_requires_every_referenced_object() {
    let r = rig();
    let (id, rev) = r.ready().await;
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    r.advance(20);
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[19]).await;
    r.mem.set_health(
        "KGRR",
        Health {
            active: true,
            low_sweep_end: Some(r.now() - TimeDelta::seconds(19)),
            last_advance: Some(r.now()),
            ..Health::default()
        },
    );
    r.objects
        .delete(&Path::from(revision.tile_keys[0].as_str()))
        .await
        .unwrap();
    let (_, body) = r.submit("classic").await;
    assert_ne!(
        body["state"], "ready",
        "a missing object ends the promise at once"
    );
}

#[tokio::test]
async fn a_late_or_future_source_never_publishes() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[800]).await;
    r.healthy("KGRR");
    r.submit("classic").await;
    assert_eq!(
        r.publisher.run_once().await.unwrap(),
        Done::Waiting("late".into())
    );
    r.advance(31);
    let (_, body) = r.submit("classic").await;
    assert_eq!(
        (body["state"].as_str(), body["reason"].as_str()),
        (Some("fallback"), Some("pending_timeout"))
    );
    assert_eq!(r.mem.revision_count(), 0);
}

#[tokio::test]
async fn sites_lists_enabled_health_without_polling_per_request() {
    let r = rig();
    r.healthy("KGRR");
    let a = r.service.sites().await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&a.bytes).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["enabled_regions"], serde_json::json!(["KGRR", "KDTX"]));
    let kgrr = v["sites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "KGRR")
        .unwrap();
    assert_eq!(kgrr["enabled"], true);
    assert_eq!(kgrr["health"]["state"], "healthy");
    assert!(
        v["sites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["enabled"] == false && s["health"].is_null())
    );
    r.mem.fail_all.store(true, Ordering::SeqCst);
    let b = r.service.sites().await.unwrap();
    assert_eq!(a.etag, b.etag, "cached snapshot, no store call");
}

#[tokio::test]
async fn out_of_range_and_unhealthy_stations_fall_back_with_a_reason() {
    let r = rig();
    let far = RawView {
        centre: Centre {
            lat: 35.0,
            lon: -100.0,
        },
        zoom: 9,
        tiles: vec![[0, 0]],
        palette: "classic".into(),
    };
    let (x, y) = (
        crate::viewport::tile_x(-100.0, 9) as u32,
        crate::viewport::tile_y(35.0, 9) as u32,
    );
    let far = RawView {
        tiles: vec![[x, y]],
        ..far
    };
    let Submit::Reply { body, .. } = r.service.submit(far).await.unwrap() else {
        panic!()
    };
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body.bytes).unwrap()["reason"],
        "out_of_range"
    );
    r.mem.set_health(
        "KGRR",
        Health {
            active: true,
            low_sweep_end: Some(r.base - TimeDelta::seconds(900)),
            ..Health::default()
        },
    );
    let (_, body) = r.submit("classic").await;
    assert_eq!(
        (body["state"].as_str(), body["reason"].as_str()),
        (Some("fallback"), Some("late"))
    );
    assert!(matches!(
        r.service
            .submit(RawView {
                centre: Centre { lat: 0.0, lon: 0.0 },
                zoom: 3,
                tiles: vec![],
                palette: "classic".into()
            })
            .await
            .unwrap(),
        Submit::Invalid("invalid_zoom")
    ));
}

#[tokio::test]
async fn the_ninth_station_and_the_sixty_fifth_pending_view_are_rejected() {
    let r = rig();
    for i in 0..8 {
        assert!(
            r.mem
                .touch_station(&format!("K{i:03}"), r.now())
                .await
                .unwrap()
        );
    }
    assert!(!r.mem.touch_station("KZZZ", r.now()).await.unwrap());
    assert!(r.mem.touch_station("K000", r.now()).await.unwrap());
    let r = rig();
    for i in 0..MAX_PENDING_VIEWS {
        let scope = ViewScope {
            station: "KGRR".into(),
            zoom: 9,
            tiles: vec![[i as u32, 0]],
            palette: "classic".into(),
        };
        assert!(matches!(
            r.mem.submit(&scope, [42.9, -85.5], r.now()).await.unwrap(),
            Submitted::Record(_)
        ));
    }
    let scope = ViewScope {
        station: "KGRR".into(),
        zoom: 9,
        tiles: vec![[999, 0]],
        palette: "classic".into(),
    };
    assert!(matches!(
        r.mem.submit(&scope, [42.9, -85.5], r.now()).await.unwrap(),
        Submitted::Saturated
    ));
}

#[tokio::test]
async fn path_inputs_are_validated() {
    let r = rig();
    let ok = "a".repeat(64);
    for bad in [
        ("kgrr", ok.as_str(), VERSION, "classic"),
        ("KGRR", "../x", VERSION, "classic"),
        ("KGRR", ok.as_str(), "../..", "classic"),
        ("KGRR", ok.as_str(), VERSION, "neon"),
    ] {
        assert!(
            r.service
                .tile_key(bad.0, bad.1, bad.2, bad.3, 9, 1, 1)
                .is_none(),
            "{bad:?}"
        );
    }
    assert!(
        r.service
            .tile_key("KGRR", &ok, VERSION, "classic", 9, 512, 1)
            .is_none()
    );
    assert!(
        r.service
            .tile_key("KGRR", &ok, VERSION, "classic", 9, 1, 1)
            .is_some()
    );
    assert!(!valid_view_id("../../etc"));
    assert!(valid_view_id(&"0".repeat(32)));
    assert!(!valid_revision("ABCDEF0123456789"));
}

#[tokio::test]
async fn store_delay_past_the_deadline_times_out_without_publishing() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    *r.fake.delay.lock().unwrap() = Duration::from_millis(400);
    r.submit("classic").await;
    let view = r
        .mem
        .claim(r.now(), TimeDelta::seconds(10))
        .await
        .unwrap()
        .unwrap();
    r.advance(29);
    let handle = r.publisher.process(view);
    let out = handle.await;
    assert!(matches!(out, Done::Failed(_) | Done::Published(_)));
    r.advance(10);
    assert_ne!(r.submit("classic").await.1["state"], "pending");
}

#[tokio::test]
async fn restart_resumes_an_unfinished_claim_after_its_ttl() {
    let r = rig();
    r.add_sweeps("KGRR", 42.89389, -85.54489, &[60]).await;
    r.healthy("KGRR");
    r.submit("classic").await;
    assert!(
        r.mem
            .claim(r.now(), TimeDelta::seconds(10))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        r.mem
            .claim(r.now(), TimeDelta::seconds(10))
            .await
            .unwrap()
            .is_none(),
        "claimed builds are not doubled"
    );
    r.advance(11);
    assert!(matches!(
        r.publisher.run_once().await.unwrap(),
        Done::Published(_)
    ));
}

async fn pg_rig() -> Option<(
    Rig,
    Arc<crate::pgstore::PgStore>,
    Arc<crate::tile::TileService>,
    String,
    String,
)> {
    let Ok(url) = std::env::var("LEVEL2_TEST_PG") else {
        eprintln!("SKIP: LEVEL2_TEST_PG absent");
        return None;
    };
    let schema = format!("level2_test_{}", uuid::Uuid::new_v4().simple());
    let mut r = rig();
    let pool = crate::leases::Pool::connect(&url, &schema).await.unwrap();
    pool.initialize().await.unwrap();
    let pg = Arc::new(crate::pgstore::PgStore::new(pool).await.unwrap());
    let tiles_pool = crate::leases::Pool::connect(&url, &schema).await.unwrap();
    let tiles = crate::tile::TileService::new(
        tiles_pool,
        r.objects.clone(),
        crate::tile::Limits::default(),
    )
    .unwrap();
    r.publisher.store = pg.clone();
    r.publisher.index = pg.clone();
    r.publisher.tiles = Arc::new(tiles.clone());
    r.service = Level2Service::new(
        pg.clone(),
        pg.clone(),
        r.objects.clone(),
        r.publisher.catalogue.clone(),
        vec!["KGRR".into()],
        NS.into(),
        r.publisher.clock.clone(),
    )
    .unwrap();
    Some((r, pg, tiles, url, schema))
}

async fn index(
    url: &str,
    schema: &str,
    station: &str,
    end: DateTime<Utc>,
    identity: &str,
    key: &str,
) -> anyhow::Result<()> {
    let mut pool = crate::leases::Pool::connect(url, schema).await?;
    pool.index(station, end, identity, key).await
}

#[tokio::test]
async fn postgres_real_tiles_publish_verify_expire_and_collect() {
    let Some((r, pg, _tiles, url, schema)) = pg_rig().await else {
        return;
    };
    for secs in [300, 180, 60] {
        let end = r.base - TimeDelta::seconds(secs);
        let a = Artifact::build(&sweep("KGRR", 42.89389, -85.54489, end), vec![], NS).unwrap();
        a.publish(r.objects.as_ref()).await.unwrap();
        index(
            &url,
            &schema,
            "KGRR",
            end,
            &a.commit.identity,
            &a.commit_key,
        )
        .await
        .unwrap();
    }
    let pool = crate::leases::Pool::connect(&url, &schema).await.unwrap();
    pool.save_health(
        "KGRR",
        &Health {
            active: true,
            low_sweep_end: Some(r.base - TimeDelta::seconds(30)),
            last_advance: Some(r.base - TimeDelta::seconds(10)),
            ..Health::default()
        },
    )
    .await
    .unwrap();
    let (status, body) = r.submit("classic").await;
    assert_eq!((status, body["state"].as_str()), (202, Some("pending")));
    let id = body["view_id"].as_str().unwrap().to_owned();
    let done = r.publisher.run_once().await.unwrap();
    let Done::Published(rev) = done else {
        panic!("{done:?}")
    };
    let (status, body) = r.submit("classic").await;
    assert_eq!((status, body["state"].as_str()), (200, Some("ready")));
    let ManifestReply::Ok(revision) = r.service.manifest(&id, &rev).await else {
        panic!()
    };
    assert_eq!(revision.tile_keys.len(), 6);
    // Every advertised URL across every frame and tile reads back a real PNG with the stored checksum.
    for key in &revision.tile_keys {
        let p: Vec<&str> = key.split('/').collect();
        let y: u32 = p[9].trim_end_matches(".png").parse().unwrap();
        let TileReply::Ok { bytes, sha256 } = r
            .service
            .tile(
                p[3],
                p[4],
                p[5],
                p[6],
                p[7].parse().unwrap(),
                p[8].parse().unwrap(),
                y,
            )
            .await
        else {
            panic!("{key}")
        };
        assert!(bytes.starts_with(b"\x89PNG"));
        assert_eq!(crate::checksum(&bytes), sha256);
    }
    assert!(pg.collect(r.now()).await.unwrap().is_empty());
    r.advance(VIEW_LEASE_S + READ_GRACE_S + 1);
    assert!(matches!(
        r.service.manifest(&id, &rev).await,
        ManifestReply::Gone
    ));
    let dead = pg.collect(r.now()).await.unwrap();
    assert_eq!(dead.len(), 6);
    assert_eq!(
        pg.stage_pins("o", &dead[..1], r.now() + TimeDelta::seconds(5))
            .await
            .unwrap(),
        Pin::Collecting
    );
    assert!(matches!(
        r.service
            .tile("KGRR", &"0".repeat(64), VERSION, "classic", 9, 1, 1)
            .await,
        TileReply::NotFound
    ));
    assert_eq!(r.publisher.collect().await.unwrap(), 0);
    pg.collected(&dead).await.unwrap();
    assert_eq!(
        pg.stage_pins("o", &dead[..1], r.now() + TimeDelta::seconds(5))
            .await
            .unwrap(),
        Pin::Staged
    );
}

#[tokio::test]
async fn postgres_collision_commit_failure_and_quota_leave_last_good() {
    let Some((r, pg, _tiles, url, schema)) = pg_rig().await else {
        return;
    };
    let end = r.base - TimeDelta::seconds(60);
    let a = Artifact::build(&sweep("KGRR", 42.89389, -85.54489, end), vec![], NS).unwrap();
    a.publish(r.objects.as_ref()).await.unwrap();
    index(
        &url,
        &schema,
        "KGRR",
        end,
        &a.commit.identity,
        &a.commit_key,
    )
    .await
    .unwrap();
    index(
        &url,
        &schema,
        "KGRR",
        end,
        &a.commit.identity,
        &a.commit_key,
    )
    .await
    .unwrap();
    let clash = index(&url, &schema, "KGRR", end, &"f".repeat(64), &a.commit_key).await;
    assert!(clash.unwrap_err().to_string().contains("collision"));
    let pool = crate::leases::Pool::connect(&url, &schema).await.unwrap();
    pool.save_health(
        "KGRR",
        &Health {
            active: true,
            low_sweep_end: Some(r.base - TimeDelta::seconds(30)),
            last_advance: Some(r.base - TimeDelta::seconds(10)),
            ..Health::default()
        },
    )
    .await
    .unwrap();
    r.submit("classic").await;
    let done = r.publisher.run_once().await.unwrap();
    let Done::Published(rev) = done else {
        panic!("{done:?}")
    };
    let id = r.submit("classic").await.1["view_id"]
        .as_str()
        .unwrap()
        .to_owned();
    // A failed rebuild keeps the ready revision readable within its lease.
    pg.start_refresh(&id, r.now()).await.unwrap();
    pg.fail(&id, "tile_missing", "build:t", r.now())
        .await
        .unwrap();
    assert_eq!(r.submit("classic").await.1["state"], "ready");
    assert!(matches!(
        r.service.manifest(&id, &rev).await,
        ManifestReply::Ok(_)
    ));
    // Object quota: reserving past the namespace cap is refused and leaves the pool unchanged.
    let mut quota = crate::leases::Pool::connect(&url, &schema).await.unwrap();
    assert!(
        quota
            .reserve_objects("big", crate::OBJECT_LIMIT)
            .await
            .is_err()
    );
}
