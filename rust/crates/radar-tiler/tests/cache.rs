//! Geometry cache behaviour (spec 15.1). These tests share the process-global cache counters
//! through `tile_geometry_cache_stats`, so they live in their own test binary and run as one
//! sequential test. The bounded and single-flight checks use private cache instances.

mod common;

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use common::*;
use radar_tiler::cache::{
    GeometryCache, GeometryKey, clear_tile_geometry_cache, tile_geometry_cache_stats,
};
use radar_tiler::geometry::{GeometryBody, Sampling, SourceGeometry, TileGeometry};
use radar_tiler::identity::{GridIdentity, grid_spec_digest};
use radar_tiler::{FrameKind, FrameOptions, Grid, GridGeo, render_frame_palettes};

fn field(h: usize, w: usize, lons_override: Option<(usize, f64)>) -> Grid {
    let lats: Vec<f64> = (0..h).map(|i| 30.0 + 0.02 * i as f64).collect();
    let mut lons: Vec<f64> = (0..w).map(|j| -100.0 + 0.02 * j as f64).collect();
    if let Some((j, v)) = lons_override {
        lons[j] = v;
    }
    let values = (0..h * w)
        .map(|k| if (k / w) % 9 < 4 { 30.0 } else { f32::NAN })
        .collect();
    Grid {
        width: w,
        height: h,
        values,
        geo: GridGeo::Regular { lats, lons },
    }
}

fn identity_of(g: &Grid) -> Arc<GridIdentity> {
    Arc::new(SourceGeometry::from_grid(g).unwrap().identity().clone())
}

fn key(id: &Arc<GridIdentity>, z: u32, tx: u32, ty: u32) -> GeometryKey {
    GeometryKey {
        grid: Arc::clone(id),
        sampling: Sampling::Bilinear,
        tile_size: 256,
        z,
        tx,
        ty,
    }
}

fn geometry() -> TileGeometry {
    TileGeometry {
        body: GeometryBody::RegularBilinear {
            rows: vec![0.0; 256],
            cols: vec![0.0; 256],
        },
        row_bounds: (0, 0),
        col_bounds: (0, 0),
    }
}

#[test]
fn geometry_cache_hits_on_second_frame_and_misses_on_new_grid() {
    // test_geometry_cache_hits_on_second_frame_and_misses_on_new_grid and
    // test_geometry_cache_skips_pyproj_on_cache_hit (adapted: the transform counter is not
    // exposed, so "no transforms on a hit" is observed as "no new misses").
    clear_tile_geometry_cache();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = [(
        "classic".to_string(),
        palette_table("classic", "reflectivity"),
    )];
    let tmp = tempfile::tempdir().unwrap();
    let first = field(120, 180, None);
    let run = |root: &str, g: &Grid| {
        let out = [("classic".to_string(), tmp.path().join(root))];
        render_frame_palettes(g, &kind, &palettes, &out, &[4, 5], &FrameOptions::default())
            .unwrap();
    };
    run("a", &first);
    let s1 = tile_geometry_cache_stats();
    assert_eq!(s1.hits, 0);
    assert!(s1.misses > 0);
    assert_eq!(s1.entries as u64, s1.misses);

    run("b", &first);
    let s2 = tile_geometry_cache_stats();
    assert_eq!(
        s2.misses, s1.misses,
        "an identical frame must not add misses"
    );
    assert_eq!(s2.hits, s1.misses, "every key of the first frame is a hit");

    // A coarser grid over the same area is a different grid (fewer, wider cells).
    let coarser = {
        let lats: Vec<f64> = (0..60).map(|i| 30.0 + 0.04 * i as f64).collect();
        let lons: Vec<f64> = (0..90).map(|j| -100.0 + 0.04 * j as f64).collect();
        Grid {
            width: 90,
            height: 60,
            values: vec![30.0; 60 * 90],
            geo: GridGeo::Regular { lats, lons },
        }
    };
    run("c", &coarser);
    let s3 = tile_geometry_cache_stats();
    assert!(
        s3.misses > s2.misses,
        "a subsampled grid is a different grid"
    );

    let perturbed = field(120, 180, Some((50, -100.0 + 0.02 * 50.0 + 1e-6)));
    run("d", &perturbed);
    let s4 = tile_geometry_cache_stats();
    assert!(
        s4.misses > s3.misses,
        "a changed interior longitude is a miss"
    );
    clear_tile_geometry_cache();
    assert_eq!(tile_geometry_cache_stats().entries, 0);
}

#[test]
fn grid_spec_fingerprints_full_axes_not_only_shape_and_endpoints() {
    // test_grid_spec_fingerprints_full_axes_not_only_shape_and_endpoints
    let base = field(120, 180, None);
    let perturbed = field(120, 180, Some((50, -100.0 + 0.02 * 50.0 + 1e-6)));
    let a = grid_spec_digest(SourceGeometry::from_grid(&base).unwrap().identity());
    let b = grid_spec_digest(SourceGeometry::from_grid(&perturbed).unwrap().identity());
    assert_ne!(a, b);
    let again = grid_spec_digest(
        SourceGeometry::from_grid(&field(120, 180, None))
            .unwrap()
            .identity(),
    );
    assert_eq!(a, again);
}

#[test]
fn geometry_cache_is_bounded_and_keeps_the_last_entry() {
    // test_geometry_cache_is_bounded: a zero byte budget and five puts leave one entry.
    let id = identity_of(&field(120, 180, None));
    let cache = GeometryCache::with_limits(0, 4096);
    for tx in 0..5u32 {
        cache.put(key(&id, 4, tx, 0), Some(geometry()));
    }
    assert_eq!(cache.stats().entries, 1);
    assert!(cache.stats().evictions >= 4);
}

#[test]
fn geometry_cache_single_flight_computes_once() {
    // test_geometry_cache_single_flight_computes_one_value: four concurrent requests cause one
    // compute, three waits, and the same shared value.
    let id = identity_of(&field(120, 180, None));
    let cache = Arc::new(GeometryCache::with_limits(64 << 20, 4096));
    let computes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let barrier = Arc::new(Barrier::new(4));
    let mut handles = Vec::new();
    for _ in 0..4 {
        let (cache, computes, barrier, id) = (
            Arc::clone(&cache),
            Arc::clone(&computes),
            Arc::clone(&barrier),
            Arc::clone(&id),
        );
        handles.push(thread::spawn(move || {
            barrier.wait();
            cache
                .get_or_compute(key(&id, 4, 3, 5), || {
                    computes.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(150));
                    Ok(Some(geometry()))
                })
                .unwrap()
                .unwrap()
        }));
    }
    let values: Vec<Arc<TileGeometry>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(computes.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(cache.stats().waits, 3);
    assert!(values.windows(2).all(|w| Arc::ptr_eq(&w[0], &w[1])));
}
