//! Process-wide geometry cache (spec-tiler section 15.1).
//!
//! Keys are `(grid identity, sampling, tile size, z, tx, ty)`. Values are `Option<TileGeometry>`:
//! `None` (no overlap) is cached too. Eviction is LRU and bounded by both array bytes and
//! entry count (never evicting the last entry). Fills are single-flight: concurrent requests
//! for one key compute it once and the others wait and then read the cached value.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use crate::env;
use crate::error::Result;
use crate::geometry::{Sampling, TileGeometry};
use crate::identity::GridIdentity;

/// Cache key for one tile geometry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GeometryKey {
    /// Source grid identity (coordinates and bounds).
    pub grid: Arc<GridIdentity>,
    /// Sampling mode.
    pub sampling: Sampling,
    /// Tile edge length in pixels.
    pub tile_size: usize,
    /// Zoom level.
    pub z: u32,
    /// Tile column.
    pub tx: u32,
    /// Tile row.
    pub ty: u32,
}

/// Counters reported by [`tile_geometry_cache_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GeometryCacheStats {
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that computed a value.
    pub misses: u64,
    /// Lookups that waited for another thread's in-flight computation.
    pub waits: u64,
    /// Entries removed to honour the budgets.
    pub evictions: u64,
    /// Entries currently held.
    pub entries: usize,
    /// Array bytes currently held.
    pub bytes: usize,
}

struct Slot {
    value: Option<Arc<TileGeometry>>,
    tick: u64,
    nbytes: usize,
}

#[derive(Default)]
struct Flight {
    done: Mutex<bool>,
    cv: Condvar,
}

impl Flight {
    fn finish(&self) {
        *self.done.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cv.notify_all();
    }

    fn wait(&self) {
        let mut done = self.done.lock().unwrap_or_else(|e| e.into_inner());
        while !*done {
            done = self.cv.wait(done).unwrap_or_else(|e| e.into_inner());
        }
    }
}

#[derive(Default)]
struct Inner {
    map: HashMap<GeometryKey, Slot>,
    order: BTreeMap<u64, GeometryKey>,
    tick: u64,
    bytes: usize,
    inflight: HashMap<GeometryKey, Arc<Flight>>,
    stats: GeometryCacheStats,
}

/// Bounded LRU cache of tile geometries with single-flight fills.
pub struct GeometryCache {
    max_bytes: usize,
    max_entries: usize,
    inner: Mutex<Inner>,
}

impl GeometryCache {
    /// Create a cache with explicit budgets. `max_entries` is at least 1.
    pub fn with_limits(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            max_bytes,
            max_entries: max_entries.max(1),
            inner: Mutex::new(Inner::default()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Return the cached geometry for `key`, computing it with `compute` on a miss.
    ///
    /// A failing `compute` is not cached; waiters retry and one of them computes.
    pub fn get_or_compute<F>(
        &self,
        key: GeometryKey,
        compute: F,
    ) -> Result<Option<Arc<TileGeometry>>>
    where
        F: FnOnce() -> Result<Option<TileGeometry>>,
    {
        let flight = loop {
            let mut g = self.lock();
            if let Some(slot) = g.map.get(&key) {
                let value = slot.value.clone();
                let old = slot.tick;
                g.tick += 1;
                let tick = g.tick;
                g.map.get_mut(&key).expect("present").tick = tick;
                g.order.remove(&old);
                g.order.insert(tick, key.clone());
                g.stats.hits += 1;
                return Ok(value);
            }
            if let Some(flight) = g.inflight.get(&key).cloned() {
                g.stats.waits += 1;
                drop(g);
                flight.wait();
                continue;
            }
            let flight = Arc::new(Flight::default());
            g.inflight.insert(key.clone(), flight.clone());
            g.stats.misses += 1;
            break flight;
        };

        // Clears the in-flight entry and wakes waiters even if `compute` panics.
        struct Guard<'a> {
            cache: &'a GeometryCache,
            key: &'a GeometryKey,
            flight: Arc<Flight>,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.cache.lock().inflight.remove(self.key);
                self.flight.finish();
            }
        }
        let guard = Guard {
            cache: self,
            key: &key,
            flight,
        };
        let result = compute();
        let out = match result {
            Ok(geom) => {
                let value = geom.map(Arc::new);
                let mut g = self.lock();
                g.inflight.remove(&key);
                self.insert_locked(&mut g, key.clone(), value.clone());
                Ok(value)
            }
            Err(e) => Err(e),
        };
        drop(guard);
        out
    }

    fn insert_locked(&self, g: &mut Inner, key: GeometryKey, value: Option<Arc<TileGeometry>>) {
        if g.map.contains_key(&key) {
            return;
        }
        let nbytes = value.as_ref().map_or(0, |v| v.nbytes());
        g.tick += 1;
        let tick = g.tick;
        g.order.insert(tick, key.clone());
        g.map.insert(
            key,
            Slot {
                value,
                tick,
                nbytes,
            },
        );
        g.bytes += nbytes;
        while (g.bytes > self.max_bytes || g.map.len() > self.max_entries) && g.map.len() > 1 {
            let Some((_, oldest)) = g.order.pop_first() else {
                break;
            };
            if let Some(slot) = g.map.remove(&oldest) {
                g.bytes -= slot.nbytes;
            }
            g.stats.evictions += 1;
        }
    }

    /// Insert a value directly (used by tests of the budget rules).
    pub fn put(&self, key: GeometryKey, value: Option<TileGeometry>) {
        let mut g = self.lock();
        self.insert_locked(&mut g, key, value.map(Arc::new));
    }

    /// Current counters.
    pub fn stats(&self) -> GeometryCacheStats {
        let g = self.lock();
        GeometryCacheStats {
            entries: g.map.len(),
            bytes: g.bytes,
            ..g.stats
        }
    }

    /// Drop every entry and reset the counters.
    pub fn clear(&self) {
        let mut g = self.lock();
        g.map.clear();
        g.order.clear();
        g.bytes = 0;
        g.stats = GeometryCacheStats::default();
    }
}

static GLOBAL: OnceLock<GeometryCache> = OnceLock::new();

/// The process-wide cache. Budgets come from `TILE_GEOMETRY_CACHE_MB` and
/// `TILE_GEOMETRY_CACHE_ENTRIES`, read once on first use.
pub fn global_cache() -> &'static GeometryCache {
    GLOBAL.get_or_init(|| {
        let mb = env::geometry_cache_mb() as usize;
        let entries = env::geometry_cache_entries() as usize;
        GeometryCache::with_limits(mb * 1024 * 1024, entries)
    })
}

/// Statistics of the process-wide cache (`tile_geometry_cache_stats`).
pub fn tile_geometry_cache_stats() -> GeometryCacheStats {
    global_cache().stats()
}

/// Reset the process-wide cache (`clear_tile_geometry_cache`).
pub fn clear_tile_geometry_cache() {
    global_cache().clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::GeometryBody;
    use crate::identity::{CoordinateIdentity, fingerprint_f64};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn grid(tag: f64) -> Arc<GridIdentity> {
        let fp = fingerprint_f64(&[tag, 1.0]);
        Arc::new(GridIdentity {
            kind: "regular",
            height: 2,
            width: 2,
            lat_bounds: (0.0, 1.0),
            lon_bounds: (0.0, 1.0),
            coordinates: CoordinateIdentity::Regular {
                lats: fp.clone(),
                lons: fp,
            },
        })
    }

    fn key(i: u32) -> GeometryKey {
        GeometryKey {
            grid: grid(0.0),
            sampling: Sampling::Nearest,
            tile_size: 256,
            z: 4,
            tx: i,
            ty: 0,
        }
    }

    fn geom() -> TileGeometry {
        TileGeometry {
            body: GeometryBody::RegularNearest {
                rows: vec![0; 256],
                cols: vec![0; 256],
            },
            row_bounds: (0, 0),
            col_bounds: (0, 0),
        }
    }

    /// test_geometry_cache_is_bounded: budget 0 and five puts leave one entry.
    #[test]
    fn budget_zero_keeps_only_the_newest_entry() {
        let c = GeometryCache::with_limits(0, 4096);
        for i in 0..5 {
            c.put(key(i), Some(geom()));
        }
        assert_eq!(c.stats().entries, 1);
        assert_eq!(c.stats().evictions, 4);
    }

    #[test]
    fn entry_budget_evicts_oldest_first() {
        let c = GeometryCache::with_limits(usize::MAX, 2);
        c.put(key(0), None);
        c.put(key(1), None);
        let _ = c.get_or_compute(key(0), || Ok(None)).unwrap(); // touch 0 so 1 is the oldest
        c.put(key(2), None);
        let s = c.stats();
        assert_eq!(s.entries, 2);
        let calls = AtomicUsize::new(0);
        let _ = c.get_or_compute(key(1), || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1, "key 1 was evicted");
    }

    /// test_geometry_cache_single_flight_computes_one_value: four concurrent requests, one compute.
    #[test]
    fn single_flight_computes_once_and_shares_the_value() {
        let cache = Arc::new(GeometryCache::with_limits(usize::MAX, 16));
        let calls = Arc::new(AtomicUsize::new(0));
        let k = key(9);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let cache = cache.clone();
                let calls = calls.clone();
                let k = k.clone();
                std::thread::spawn(move || {
                    cache
                        .get_or_compute(k, move || {
                            calls.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(150));
                            Ok(Some(geom()))
                        })
                        .unwrap()
                        .unwrap()
                })
            })
            .collect();
        let results: Vec<Arc<TileGeometry>> =
            handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|r| Arc::ptr_eq(r, &results[0])));
        let s = cache.stats();
        assert_eq!(s.misses, 1);
        assert_eq!(s.waits, 3);
    }

    #[test]
    fn failing_compute_is_not_cached() {
        let c = GeometryCache::with_limits(usize::MAX, 16);
        let err = c.get_or_compute(key(3), || {
            Err(crate::error::TilerError::Invalid("boom".into()))
        });
        assert!(err.is_err());
        assert_eq!(c.stats().entries, 0);
        let ok = c.get_or_compute(key(3), || Ok(Some(geom()))).unwrap();
        assert!(ok.is_some());
    }

    #[test]
    fn negative_results_are_cached() {
        let c = GeometryCache::with_limits(usize::MAX, 16);
        assert!(c.get_or_compute(key(4), || Ok(None)).unwrap().is_none());
        let calls = AtomicUsize::new(0);
        assert!(
            c.get_or_compute(key(4), || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(None)
            })
            .unwrap()
            .is_none()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(c.stats().hits, 1);
    }

    #[test]
    fn stats_track_hits_misses_and_clear_resets() {
        let c = GeometryCache::with_limits(usize::MAX, 16);
        c.put(key(5), Some(geom()));
        let _ = c.get_or_compute(key(5), || Ok(None)).unwrap();
        assert_eq!(c.stats().hits, 1);
        c.clear();
        assert_eq!(c.stats(), GeometryCacheStats::default());
    }
}
