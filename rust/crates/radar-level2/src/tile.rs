use crate::{
    HOT_LIMIT, MIB, RENDER_LIMIT, artifact,
    leases::{Pool, Reservation},
    polar::{
        PolarSweep, SAMPLER_VERSION, Scratch, TILE_SCRATCH_BYTES, palette_version,
        reflectivity_model,
    },
};
use anyhow::Result;
use futures_util::future::{BoxFuture, FutureExt, Shared};
use object_store::{ObjectStore, path::Path};
use radar_tiler::ClassModel;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

pub const PALETTES: [&str; 3] = ["classic", "muted", "vivid"];

#[derive(Clone, Copy)]
pub struct Limits {
    pub sweep: usize,
    pub render: usize,
    pub scratch: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            sweep: HOT_LIMIT,
            render: RENDER_LIMIT,
            scratch: TILE_SCRATCH_BYTES,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TileRequest {
    pub commit_key: String,
    pub palette: String,
    pub z: u32,
    pub x: u32,
    pub y: u32,
}

#[derive(Clone, Debug)]
pub struct Ready {
    pub key: String,
    pub sha256: String,
    pub transparent: bool,
    pub source: &'static str,
    pub png: Arc<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Ready(Ready),
    Fallback(&'static str),
}

type TileEntry = (Arc<Vec<u8>>, String, bool);
#[cfg(test)]
type Latch = Arc<(Mutex<bool>, std::sync::Condvar)>;
type Flight = (
    Shared<BoxFuture<'static, Outcome>>,
    Arc<AtomicUsize>,
    Arc<AtomicBool>,
);

struct Cached<T> {
    value: T,
    id: String,
    used: u64,
}

#[derive(Default)]
struct State {
    clock: u64,
    sweeps: HashMap<String, Cached<Arc<PolarSweep>>>,
    tiles: HashMap<String, Cached<TileEntry>>,
    flights: HashMap<String, Flight>,
}

pub struct TileService {
    pool: tokio::sync::Mutex<Pool>,
    store: Arc<dyn ObjectStore>,
    model: ClassModel,
    version: String,
    limits: Limits,
    instance: String,
    state: Mutex<State>,
    loading: tokio::sync::Mutex<()>,
    peak_scratch: AtomicUsize,
    jobs: Arc<AtomicUsize>,
    #[cfg(test)]
    latch: Mutex<Option<Latch>>,
}

struct Waiter(Arc<AtomicUsize>, Arc<AtomicBool>);
impl Drop for Waiter {
    fn drop(&mut self) {
        if self.0.fetch_sub(1, Ordering::SeqCst) == 1 {
            self.1.store(true, Ordering::SeqCst);
        }
    }
}

/// `(namespace, station, identity)` from `<ns>/sweeps/<station>/<identity>/<revision>/commit.json`.
fn parse_commit_key(key: &str) -> Option<(&str, &str, &str)> {
    let (ns, rest) = key.split_once("/sweeps/")?;
    let mut parts = rest.split('/');
    let (station, identity, revision, name) =
        (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    (parts.next().is_none()
        && name == "commit.json"
        && !revision.is_empty()
        && station.len() == 4
        && station
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && identity.len() == 64
        && identity.bytes().all(|b| b.is_ascii_hexdigit()))
    .then_some((ns, station, identity))
}

fn db<T>(r: Result<T>) -> Result<T, &'static str> {
    r.map_err(|_| "database_unavailable")
}

impl TileService {
    pub fn new(pool: Pool, store: Arc<dyn ObjectStore>, limits: Limits) -> Result<Arc<Self>> {
        let model = reflectivity_model()?;
        Ok(Arc::new(Self {
            pool: tokio::sync::Mutex::new(pool),
            store,
            version: format!("{SAMPLER_VERSION}-{}", palette_version(&model)),
            model,
            limits,
            instance: uuid::Uuid::new_v4().simple().to_string(),
            state: Mutex::new(State::default()),
            loading: tokio::sync::Mutex::new(()),
            peak_scratch: AtomicUsize::new(0),
            jobs: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            latch: Mutex::new(None),
        }))
    }

    /// Blocking render jobs started by this process.
    pub fn jobs_started(&self) -> usize {
        self.jobs.load(Ordering::Relaxed)
    }

    pub fn hot_sweep_bytes(&self) -> usize {
        self.state
            .lock()
            .expect("state lock")
            .sweeps
            .values()
            .map(|c| c.value.bytes())
            .sum()
    }

    pub fn peak_scratch_bytes(&self) -> usize {
        self.peak_scratch.load(Ordering::Relaxed)
    }

    pub fn tile_key(&self, req: &TileRequest) -> Option<String> {
        let (ns, station, identity) = parse_commit_key(&req.commit_key)?;
        ((8..=13).contains(&req.z)
            && req.x < 1 << req.z
            && req.y < 1 << req.z
            && PALETTES.contains(&req.palette.as_str()))
        .then(|| {
            format!(
                "{ns}/tiles/{station}/{identity}/{}/{}/{}/{}/{}.png",
                self.version, req.palette, req.z, req.x, req.y
            )
        })
    }

    /// Renders one requested tile. Callers that time out or drop get MRMS; the job keeps its permits until its blocking work ends.
    pub async fn render(self: &Arc<Self>, req: TileRequest, deadline: Duration) -> Outcome {
        let Some(key) = self.tile_key(&req) else {
            return Outcome::Fallback("invalid_tile");
        };
        let (shared, _waiter) = {
            let mut state = self.state.lock().expect("state lock");
            let (shared, waiters, cancel) = state
                .flights
                .entry(key.clone())
                .or_insert_with(|| {
                    let (waiters, cancel) = (
                        Arc::new(AtomicUsize::new(0)),
                        Arc::new(AtomicBool::new(false)),
                    );
                    let (service, flight_key, flag) = (self.clone(), key.clone(), cancel.clone());
                    let handle = tokio::spawn(async move {
                        let outcome = service.produce(&req, &flight_key, flag).await;
                        service
                            .state
                            .lock()
                            .expect("state lock")
                            .flights
                            .remove(&flight_key);
                        outcome
                    });
                    let future: BoxFuture<'static, Outcome> = async move {
                        handle
                            .await
                            .unwrap_or(Outcome::Fallback("render_task_failed"))
                    }
                    .boxed();
                    (future.shared(), waiters, cancel)
                })
                .clone();
            waiters.fetch_add(1, Ordering::SeqCst);
            (shared, Waiter(waiters, cancel))
        };
        tokio::time::timeout(deadline, shared)
            .await
            .unwrap_or(Outcome::Fallback("render_timeout"))
    }

    async fn produce(&self, req: &TileRequest, key: &str, cancel: Arc<AtomicBool>) -> Outcome {
        if let Some(hit) = self.hot_tile(key) {
            return Outcome::Ready(hit);
        }
        let mut held = Held::default();
        let result = self.run(req, key, &cancel, &mut held).await;
        let pool = self.pool.lock().await;
        if let Some(permit) = &held.permit {
            let _ = pool.release_render(permit).await;
        }
        if let Some(id) = &held.scratch {
            let _ = pool.release(id).await;
        }
        drop(pool);
        match result {
            Ok(outcome) => Outcome::Ready(outcome),
            Err(reason) => Outcome::Fallback(reason),
        }
    }

    fn hot_tile(&self, key: &str) -> Option<Ready> {
        let mut state = self.state.lock().expect("state lock");
        state.clock += 1;
        let clock = state.clock;
        let hit = state.tiles.get_mut(key)?;
        hit.used = clock;
        Some(Ready {
            key: key.into(),
            sha256: hit.value.1.clone(),
            transparent: hit.value.2,
            source: "hot_cache",
            png: hit.value.0.clone(),
        })
    }

    async fn run(
        &self,
        req: &TileRequest,
        key: &str,
        cancel: &Arc<AtomicBool>,
        held: &mut Held,
    ) -> Result<Ready, &'static str> {
        let indexed = db(self.pool.lock().await.tile(key).await)?;
        if let Some((sha, bytes, transparent)) = indexed {
            let png = artifact::read_bounded(self.store.as_ref(), &Path::from(key), bytes as usize)
                .await
                .map_err(|_| "tile_unavailable")?;
            if crate::checksum(&png) != sha || png.len() as i64 != bytes {
                return Err("tile_corrupt");
            }
            return Ok(Ready {
                key: key.into(),
                sha256: sha,
                transparent,
                source: "rustfs",
                png: Arc::new(png),
            });
        }
        let permit = db(self.pool.lock().await.render_permit().await)?.ok_or("render_saturated")?;
        held.permit = Some(permit);
        let id = format!("tile:{}:{key}", self.instance);
        self.reserve(
            "render",
            &id,
            self.limits.scratch,
            self.limits.render,
            |s| s.evict_tile(),
        )
        .await?;
        held.scratch = Some(id.clone());
        let sweep = self.sweep(&req.commit_key).await?;
        let blocking_sweep = sweep.clone();
        let (model_ref, flag) = (self.model.clone(), cancel.clone());
        let (z, x, y, palette) = (req.z, req.x, req.y, req.palette.clone());
        #[cfg(test)]
        let latch = self.latch.lock().expect("latch").clone();
        let jobs = self.jobs.clone();
        let job = tokio::task::spawn_blocking(move || {
            jobs.fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            if let Some(latch) = latch {
                let (lock, cv) = &*latch;
                let mut open = lock.lock().expect("latch lock");
                while !*open {
                    open = cv.wait(open).expect("latch wait");
                }
            }
            let mut scratch = Scratch::default();
            let rendered =
                blocking_sweep.render_tile(z, x, y, &model_ref, &palette, &flag, &mut scratch);
            rendered.map(|r| (r, scratch.bytes()))
        });
        let (rendered, scratch_bytes) = match job.await {
            Ok(Ok(done)) => done,
            Ok(Err(_)) if cancel.load(Ordering::SeqCst) => return Err("render_cancelled"),
            _ => return Err("render_failed"),
        };
        self.peak_scratch
            .fetch_max(scratch_bytes, Ordering::Relaxed);
        if scratch_bytes + rendered.png.len() > self.limits.scratch {
            return Err("render_oversized");
        }
        let png = Arc::new(rendered.png);
        let sha = crate::checksum(&png);
        db(self
            .pool
            .lock()
            .await
            .reserve_objects(key, png.len() as i64)
            .await)?;
        write_immutable(self.store.as_ref(), key, &png)
            .await
            .map_err(|_| "store_write_failed")?;
        {
            let pool = self.pool.lock().await;
            db(pool
                .put_tile(key, &sha, png.len() as i64, rendered.transparent)
                .await)?;
            db(pool.shrink(&id, png.len() as i64).await)?;
        }
        held.scratch = None;
        let mut state = self.state.lock().expect("state lock");
        state.clock += 1;
        let used = state.clock;
        state.tiles.insert(
            key.into(),
            Cached {
                value: (png.clone(), sha.clone(), rendered.transparent),
                id,
                used,
            },
        );
        Ok(Ready {
            key: key.into(),
            sha256: sha,
            transparent: rendered.transparent,
            source: "rendered",
            png,
        })
    }

    /// Reserves pool budget, evicting this process's own idle entries before giving up.
    async fn reserve(
        &self,
        kind: &str,
        id: &str,
        bytes: usize,
        cap: usize,
        evict: impl Fn(&Self) -> Option<String>,
    ) -> Result<(), &'static str> {
        loop {
            let outcome = db(self
                .pool
                .lock()
                .await
                .reserve(kind, id, bytes as i64, cap as i64)
                .await)?;
            match outcome {
                Reservation::Granted => return Ok(()),
                Reservation::Duplicate => return Err("reservation_conflict"),
                Reservation::Full => {
                    let victim = evict(self).ok_or(if kind == "sweep" {
                        "sweep_cache_full"
                    } else {
                        "render_cache_full"
                    })?;
                    db(self.pool.lock().await.release(&victim).await)?;
                }
            }
        }
    }

    fn evict_tile(&self) -> Option<String> {
        let mut state = self.state.lock().expect("state lock");
        let key = state
            .tiles
            .iter()
            .min_by_key(|(_, c)| c.used)
            .map(|(k, _)| k.clone())?;
        state.tiles.remove(&key).map(|c| c.id)
    }

    fn evict_sweep(&self) -> Option<String> {
        let mut state = self.state.lock().expect("state lock");
        let key = state
            .sweeps
            .iter()
            .filter(|(_, c)| Arc::strong_count(&c.value) == 1)
            .min_by_key(|(_, c)| c.used)
            .map(|(k, _)| k.clone())?;
        state.sweeps.remove(&key).map(|c| c.id)
    }

    async fn sweep(&self, commit_key: &str) -> Result<Arc<PolarSweep>, &'static str> {
        let touch = |s: &Self| {
            let mut state = s.state.lock().expect("state lock");
            state.clock += 1;
            let clock = state.clock;
            state.sweeps.get_mut(commit_key).map(|c| {
                c.used = clock;
                c.value.clone()
            })
        };
        if let Some(sweep) = touch(self) {
            return Ok(sweep);
        }
        let _loading = self.loading.lock().await;
        if let Some(sweep) = touch(self) {
            return Ok(sweep);
        }
        let commit = artifact::read_commit(self.store.as_ref(), commit_key)
            .await
            .map_err(|_| "artifact_unavailable")?;
        let (_, station, identity) = parse_commit_key(commit_key).ok_or("artifact_mismatch")?;
        if commit.station != station || commit.identity != identity {
            return Err("artifact_mismatch");
        }
        let peak = 3 * artifact::wanted_bytes(&commit) + MIB;
        let id = format!("sweep:{}:{commit_key}", self.instance);
        self.reserve("sweep", &id, peak, self.limits.sweep, |s| s.evict_sweep())
            .await?;
        let loaded = artifact::load(self.store.as_ref(), commit_key).await;
        let Ok((_, sweep)) = loaded else {
            let _ = self.pool.lock().await.release(&id).await;
            return Err("artifact_unavailable");
        };
        let actual = sweep.bytes();
        if actual > peak || db(self.pool.lock().await.shrink(&id, actual as i64).await).is_err() {
            let _ = self.pool.lock().await.release(&id).await;
            return Err("sweep_oversized");
        }
        let sweep = Arc::new(sweep);
        let mut state = self.state.lock().expect("state lock");
        state.clock += 1;
        let used = state.clock;
        state.sweeps.insert(
            commit_key.into(),
            Cached {
                value: sweep.clone(),
                id,
                used,
            },
        );
        Ok(sweep)
    }
}

#[derive(Default)]
struct Held {
    permit: Option<String>,
    scratch: Option<String>,
}

/// Creates the object once. A concurrent identical writer succeeds only if the bytes match.
async fn write_immutable(store: &dyn ObjectStore, key: &str, bytes: &[u8]) -> Result<()> {
    match artifact::write_verified(store, key, bytes).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let existing = artifact::read_bounded(store, &Path::from(key), bytes.len()).await;
            match existing {
                Ok(old) if crate::checksum(&old) == crate::checksum(bytes) => Ok(()),
                _ => Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests;
