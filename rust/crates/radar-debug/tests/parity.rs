//! `radar parity` on small synthetic pyramids.

use std::fs;
use std::path::{Path, PathBuf};

use radar_debug::parity::{
    CompareOptions, DEFAULT_MAX_DIFF_PCT, Filter, ParityReport, Status, compare_roots, decode_rgba,
    render_human,
};

const T1: &str = "2026-10-14T06:00:00+00:00";
const T2: &str = "2026-10-14T06:02:00+00:00";
const SIZE: u32 = 16;

/// index 0 transparent, 1 green, 2 red, 3 half-transparent blue.
const PLTE: [u8; 12] = [0, 0, 0, 10, 200, 10, 200, 10, 10, 0, 0, 255];
const TRNS: [u8; 4] = [0, 255, 255, 128];

fn tile_indices(seed: u32) -> Vec<u8> {
    (0..SIZE * SIZE)
        .map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            // A quarter of the tile stays transparent.
            if x < SIZE / 2 && y < SIZE / 2 {
                0
            } else {
                ((x + y + seed) % 3 + 1) as u8
            }
        })
        .collect()
}

fn to_rgba(indices: &[u8]) -> Vec<u8> {
    indices
        .iter()
        .flat_map(|&i| {
            let i = i as usize;
            [PLTE[3 * i], PLTE[3 * i + 1], PLTE[3 * i + 2], TRNS[i]]
        })
        .collect()
}

fn encode(path: &Path, w: u32, h: u32, data: &[u8], indexed: bool) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_depth(png::BitDepth::Eight);
    if indexed {
        enc.set_color(png::ColorType::Indexed);
        enc.set_palette(PLTE.to_vec());
        enc.set_trns(TRNS.to_vec());
    } else {
        enc.set_color(png::ColorType::Rgba);
    }
    let mut writer = enc.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
}

fn tile_path(
    root: &Path,
    layer: &str,
    palette: &str,
    frame: &str,
    z: u32,
    x: u32,
    y: u32,
) -> PathBuf {
    root.join(layer)
        .join(palette)
        .join(frame)
        .join(z.to_string())
        .join(x.to_string())
        .join(format!("{y}.png"))
}

const TILES: [(u32, u32, u32); 5] = [(3, 1, 2), (3, 2, 2), (4, 2, 4), (4, 3, 4), (4, 3, 5)];

/// Write a pyramid of [`TILES`] for `frame`, indexed or RGBA, under `layer/palette`.
fn write_pyramid(root: &Path, layer: &str, palette: &str, frame: &str, indexed: bool) {
    for (i, &(z, x, y)) in TILES.iter().enumerate() {
        let idx = tile_indices(i as u32);
        let path = tile_path(root, layer, palette, frame, z, x, y);
        if indexed {
            encode(&path, SIZE, SIZE, &idx, true);
        } else {
            encode(&path, SIZE, SIZE, &to_rgba(&idx), false);
        }
    }
    // Marker, lock dir and a staging dir with a stray tile: none of these are tiles.
    let frame_dir = root.join(layer).join(palette).join(frame);
    fs::write(frame_dir.join(".render-complete-v2.json"), "{}\n").unwrap();
    fs::create_dir_all(root.join(layer).join(".render-publish-locks")).unwrap();
    let staging = root
        .join(layer)
        .join(palette)
        .join(format!(".{frame}.tmp-0123abcd"));
    encode(&staging.join("3/1/2.png"), 8, 8, &[0u8; 64], true);
}

fn opts() -> CompareOptions {
    CompareOptions {
        filter: Filter::default(),
        max_diff_pct: DEFAULT_MAX_DIFF_PCT,
        exact: false,
        python_manifest: None,
        rust_manifest: None,
    }
}

fn exact() -> CompareOptions {
    CompareOptions {
        max_diff_pct: 0.0,
        exact: true,
        ..opts()
    }
}

struct Roots {
    _dir: tempfile::TempDir,
    py: PathBuf,
    rs: PathBuf,
}

fn roots() -> Roots {
    let dir = tempfile::tempdir().unwrap();
    let py = dir.path().join("python");
    let rs = dir.path().join("rust");
    fs::create_dir_all(&py).unwrap();
    fs::create_dir_all(&rs).unwrap();
    Roots { _dir: dir, py, rs }
}

fn both_indexed(r: &Roots) {
    for root in [&r.py, &r.rs] {
        for frame in [T1, T2] {
            write_pyramid(root, "radar", "classic", frame, true);
        }
        write_pyramid(root, "radar", "vivid", T1, true);
    }
}

fn run(r: &Roots, o: &CompareOptions) -> ParityReport {
    let report = compare_roots(&r.py, &r.rs, o).unwrap();
    // The human renderer must handle every report shape.
    assert!(render_human(&report).contains("== summary =="));
    report
}

fn rewrite(root: &Path, frame: &str, (z, x, y): (u32, u32, u32), edit: impl FnOnce(&mut Vec<u8>)) {
    let mut idx = tile_indices(TILES.iter().position(|t| *t == (z, x, y)).unwrap() as u32);
    edit(&mut idx);
    encode(
        &tile_path(root, "radar", "classic", frame, z, x, y),
        SIZE,
        SIZE,
        &idx,
        true,
    );
}

#[test]
fn identical_pyramids_pass_exact() {
    let r = roots();
    both_indexed(&r);
    for o in [opts(), exact()] {
        let rep = run(&r, &o);
        let s = &rep.summary;
        assert_eq!(s.exit_code, 0, "{:?}", s.reasons);
        assert_eq!(s.status, Status::Ok);
        assert_eq!(s.frames_compared, 3);
        assert_eq!(s.common_tiles, 15);
        assert_eq!(s.identical_tiles, 15);
        assert_eq!(s.index_compared_tiles, 15);
        assert_eq!(s.index_identical_tiles, 15);
        assert_eq!(s.pixels_compared, 15 * (SIZE * SIZE) as u64);
        assert_eq!(s.pixels_differing, 0);
        assert_eq!(s.identical_pct, Some(100.0));
        assert_eq!(s.ignored_files, [0, 0]);
        assert!(rep.frames.iter().all(|f| f.status == Status::Ok));
    }
}

#[test]
fn indexed_and_rgba_encodings_of_the_same_image_are_identical() {
    let r = roots();
    write_pyramid(&r.py, "radar", "classic", T1, false); // Python legacy-style RGBA files
    write_pyramid(&r.rs, "radar", "classic", T1, true); // Rust indexed files
    let rep = run(&r, &exact());
    let s = &rep.summary;
    assert_eq!(s.exit_code, 0, "{:?}", s.reasons);
    assert_eq!(s.identical_tiles, TILES.len());
    assert_eq!(s.index_compared_tiles, 0, "RGBA side has no indices");
    assert_eq!(s.pixels_identical, s.pixels_compared);
}

#[test]
fn decode_expands_palette_and_rgba_alike() {
    let dir = tempfile::tempdir().unwrap();
    let idx = tile_indices(1);
    let a = dir.path().join("a.png");
    let b = dir.path().join("b.png");
    encode(&a, SIZE, SIZE, &idx, true);
    encode(&b, SIZE, SIZE, &to_rgba(&idx), false);
    let da = decode_rgba(&fs::read(&a).unwrap()).unwrap();
    let db = decode_rgba(&fs::read(&b).unwrap()).unwrap();
    assert_eq!(da.rgba, db.rgba);
    assert_eq!(da.indices.as_deref(), Some(idx.as_slice()));
    assert_eq!(db.indices, None);
    assert_eq!((da.width, da.height), (SIZE, SIZE));
}

#[test]
fn missing_tile_counts_against_transparent() {
    let r = roots();
    both_indexed(&r);
    fs::remove_file(tile_path(&r.rs, "radar", "classic", T1, 4, 3, 5)).unwrap();

    // 192 of the missing tile's 256 pixels are visible; the frame has 5 * 256 pixels: 15%.
    let rep = run(
        &r,
        &CompareOptions {
            max_diff_pct: 20.0,
            ..opts()
        },
    );
    let f = rep
        .frames
        .iter()
        .find(|f| f.palette == "classic" && f.path == T1)
        .unwrap();
    assert_eq!((f.python_tiles, f.rust_tiles, f.common_tiles), (5, 4, 4));
    assert_eq!(f.only_python, ["4/3/5.png"]);
    assert_eq!(f.only_python_count, 1);
    assert_eq!(f.pixels_differing, 192);
    assert_eq!(f.diff_kinds.alpha_edge, 192);
    assert!(f.transparency_only);
    assert!((f.diff_pct - 15.0).abs() < 1e-9);
    assert_eq!(f.status, Status::Warn);
    assert_eq!(rep.summary.exit_code, 0);
    assert_eq!(rep.summary.status, Status::Warn);

    // Over the default threshold (10%): fail.
    assert_eq!(run(&r, &opts()).summary.exit_code, 1);
    // Exact: fail.
    assert_eq!(run(&r, &exact()).summary.exit_code, 1);
}

#[test]
fn missing_transparent_tile_fails_only_in_exact_mode() {
    let r = roots();
    both_indexed(&r);
    let p = tile_path(&r.py, "radar", "classic", T1, 4, 3, 5);
    encode(&p, SIZE, SIZE, &vec![0u8; (SIZE * SIZE) as usize], true);
    fs::remove_file(tile_path(&r.rs, "radar", "classic", T1, 4, 3, 5)).unwrap();
    let rep = run(
        &r,
        &CompareOptions {
            max_diff_pct: 0.0,
            ..opts()
        },
    );
    assert_eq!(rep.summary.exit_code, 0);
    assert_eq!(rep.summary.pixels_differing, 0);
    assert_eq!(rep.summary.only_python_tiles, 1);
    assert_eq!(rep.summary.status, Status::Warn);
    let rep = run(&r, &exact());
    assert_eq!(rep.summary.exit_code, 1);
    assert!(
        rep.frames
            .iter()
            .any(|f| f.problems.iter().any(|p| p.contains("tile sets differ")))
    );
}

#[test]
fn pixel_difference_is_classified_and_thresholded() {
    let r = roots();
    both_indexed(&r);
    // Colour change at (15, 15): index 2 or 3 -> 1; and a coverage edge at (8, 8): -> 0.
    rewrite(&r.rs, T1, (3, 1, 2), |idx| {
        let last = idx.len() - 1;
        idx[last] = if idx[last] == 1 { 2 } else { 1 };
        idx[(8 * SIZE + 8) as usize] = 0;
    });
    let rep = run(&r, &opts());
    let f = rep
        .frames
        .iter()
        .find(|f| f.palette == "classic" && f.path == T1)
        .unwrap();
    assert_eq!(f.pixels_differing, 2);
    assert_eq!(f.diff_kinds.color + f.diff_kinds.alpha_only, 1);
    assert_eq!(f.diff_kinds.alpha_edge, 1);
    assert!(f.max_channel_delta > 0);
    assert_eq!(f.differing_tiles, ["3/1/2.png"]);
    assert_eq!(f.identical_tiles, 4);
    assert_eq!((f.index_compared_tiles, f.index_identical_tiles), (5, 4));
    assert_eq!(f.status, Status::Warn);
    assert_eq!(rep.summary.exit_code, 0);
    assert_eq!(rep.summary.frames_warned, 1);

    let rep = run(&r, &exact());
    assert_eq!(rep.summary.exit_code, 1);
    assert_eq!(rep.summary.frames_failed, 1);
}

#[test]
fn large_difference_fails_default_threshold() {
    let r = roots();
    both_indexed(&r);
    // Recolour every visible pixel of every classic T1 tile: 75% differ.
    for &t in &TILES {
        rewrite(&r.rs, T1, t, |idx| {
            for v in idx.iter_mut().filter(|v| **v != 0) {
                *v = if *v == 1 { 2 } else { 1 };
            }
        });
    }
    let rep = run(&r, &opts());
    assert_eq!(rep.summary.exit_code, 1);
    let f = rep
        .frames
        .iter()
        .find(|f| f.status == Status::Fail)
        .unwrap();
    assert_eq!(
        (f.layer.as_str(), f.palette.as_str(), f.path.as_str()),
        ("radar", "classic", T1)
    );
    assert!((f.diff_pct - 75.0).abs() < 1e-9);
    assert!(!f.transparency_only);
    // The other frames are untouched.
    assert_eq!(rep.summary.frames_failed, 1);
}

#[test]
fn frames_on_one_side_and_nothing_to_compare() {
    let r = roots();
    write_pyramid(&r.py, "radar", "classic", T1, true);
    write_pyramid(&r.rs, "radar", "classic", T2, true);
    let rep = run(&r, &opts());
    assert_eq!(rep.summary.exit_code, 2);
    assert_eq!(rep.frames_only_python, [format!("radar/classic/{T1}")]);
    assert_eq!(rep.frames_only_rust, [format!("radar/classic/{T2}")]);

    write_pyramid(&r.rs, "radar", "classic", T1, true);
    let rep = run(&r, &opts());
    assert_eq!(rep.summary.exit_code, 0);
    assert_eq!(rep.summary.frames_compared, 1);
    assert_eq!(rep.summary.frames_only_rust, 1);
    assert_eq!(rep.summary.status, Status::Warn);
}

#[test]
fn filters_restrict_layer_palette_frame_and_zoom() {
    let r = roots();
    both_indexed(&r);
    for root in [&r.py, &r.rs] {
        write_pyramid(root, "nowcast", "classic", "lead/15", true);
        let hrrr = format!("runs/20261014_06/{T1}");
        write_pyramid(root, "hrrr", "classic", &hrrr, true);
    }
    let with = |filter: Filter| CompareOptions { filter, ..opts() };

    let rep = run(
        &r,
        &with(Filter {
            layers: vec!["radar".into()],
            ..Filter::default()
        }),
    );
    assert_eq!(rep.summary.frames_compared, 3);
    let rep = run(
        &r,
        &with(Filter {
            palettes: vec!["vivid".into()],
            ..Filter::default()
        }),
    );
    assert_eq!(rep.summary.frames_compared, 1);
    // `Z` form of the timestamp matches `+00:00` dirs, including HRRR's run-prefixed path.
    let rep = run(
        &r,
        &with(Filter {
            frames: vec!["2026-10-14T06:00:00Z".into()],
            ..Filter::default()
        }),
    );
    let names: Vec<String> = rep
        .frames
        .iter()
        .map(|f| format!("{}/{}", f.layer, f.palette))
        .collect();
    assert_eq!(names, ["hrrr/classic", "radar/classic", "radar/vivid"]);
    assert!(
        rep.frames
            .iter()
            .any(|f| f.path == format!("runs/20261014_06/{T1}"))
    );
    let rep = run(
        &r,
        &with(Filter {
            min_zoom: Some(4),
            max_zoom: Some(4),
            layers: vec!["nowcast".into()],
            ..Filter::default()
        }),
    );
    assert_eq!(rep.summary.frames_compared, 1);
    assert_eq!(rep.frames[0].path, "lead/15");
    assert_eq!(rep.summary.common_tiles, 3);
}

#[test]
fn undecodable_and_size_mismatched_tiles_fail() {
    let r = roots();
    both_indexed(&r);
    fs::write(
        tile_path(&r.rs, "radar", "classic", T1, 3, 1, 2),
        b"not a png",
    )
    .unwrap();
    encode(
        &tile_path(&r.rs, "radar", "classic", T2, 3, 1, 2),
        8,
        8,
        &[1u8; 64],
        true,
    );
    let rep = run(
        &r,
        &CompareOptions {
            max_diff_pct: 100.0,
            ..opts()
        },
    );
    assert_eq!(rep.summary.exit_code, 1);
    assert_eq!(rep.summary.decode_error_tiles, 1);
    assert_eq!(rep.summary.size_mismatch_tiles, 1);
    assert_eq!(rep.summary.frames_failed, 2);
}

fn manifest(frames: &[(&str, &str)], extra_key: Option<&str>) -> String {
    let frames: Vec<serde_json::Value> = frames
        .iter()
        .map(|(ts, source)| {
            let mut f = serde_json::json!({"timestamp": ts, "path": ts, "palettes": ["classic", "vivid"], "source": source});
            if let Some(k) = extra_key {
                f[k] = serde_json::json!(1);
            }
            f
        })
        .collect();
    let stamps: Vec<&str> = frames
        .iter()
        .map(|f| f["timestamp"].as_str().unwrap())
        .collect();
    serde_json::json!({
        "schema_version": 2,
        "tile_url_template": "/tiles/{layer}/{palette}/{path}/{z}/{x}/{y}.png",
        "updated_at": "2026-10-14T06:05:00+00:00",
        "layers": {"radar": {
            "frames": frames,
            "timestamps": stamps,
            "latest": stamps.last(),
            "palettes": ["classic", "vivid"],
            "title": "Radar",
        }},
    })
    .to_string()
}

#[test]
fn manifests_are_compared() {
    let r = roots();
    both_indexed(&r);
    // Python manifest in the root (auto-detected), Rust manifest in a state dir passed explicitly.
    fs::write(
        r.py.join("manifest.json"),
        manifest(&[(T1, "a"), (T2, "a")], None),
    )
    .unwrap();
    let state = r.rs.parent().unwrap().join("rust-state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("manifest.json"), manifest(&[(T1, "b")], None)).unwrap();
    let o = CompareOptions {
        rust_manifest: Some(state.clone()),
        ..opts()
    };

    // Frame list difference (timing) and value difference: reported, not a mismatch.
    let rep = run(&r, &o);
    let m = rep.manifest.as_ref().unwrap();
    assert!(m.errors.is_empty(), "{:?}", m.errors);
    assert_eq!(m.mismatches, 0);
    let l = &m.layers[0];
    assert_eq!((l.python_frames, l.rust_frames, l.common_frames), (2, 1, 1));
    assert_eq!(l.only_python, [T2]);
    assert_eq!(l.frame_diffs[0].values_differ, ["source"]);
    assert_eq!(rep.summary.exit_code, 0);
    let rep = run(
        &r,
        &CompareOptions {
            exact: true,
            max_diff_pct: 0.0,
            ..o.clone()
        },
    );
    assert_eq!(rep.summary.exit_code, 0);

    // A key only one side writes is an entry mismatch: warn, and fail with --exact.
    fs::write(
        state.join("manifest.json"),
        manifest(&[(T1, "a")], Some("renderer")),
    )
    .unwrap();
    let rep = run(&r, &o);
    let m = rep.manifest.as_ref().unwrap();
    assert_eq!(m.mismatches, 1);
    assert_eq!(m.layers[0].frame_diffs[0].keys_only_rust, ["renderer"]);
    assert_eq!(rep.summary.exit_code, 0);
    assert_eq!(rep.summary.status, Status::Warn);
    let rep = run(
        &r,
        &CompareOptions {
            exact: true,
            max_diff_pct: 0.0,
            ..o.clone()
        },
    );
    assert_eq!(rep.summary.exit_code, 1);

    // A malformed manifest is reported, not fatal.
    fs::write(state.join("manifest.json"), "{\"layers\": 3}").unwrap();
    let rep = run(&r, &o);
    assert_eq!(rep.manifest.as_ref().unwrap().errors.len(), 1);
    assert_eq!(rep.summary.exit_code, 0);
}

#[test]
fn json_report_has_the_documented_shape() {
    let r = roots();
    both_indexed(&r);
    let v = serde_json::to_value(run(&r, &opts())).unwrap();
    for key in [
        "python_root",
        "rust_root",
        "mode",
        "max_diff_pct",
        "frames",
        "frames_only_python",
        "frames_only_rust",
        "manifest",
        "summary",
    ] {
        assert!(v.get(key).is_some(), "missing {key}");
    }
    let f = &v["frames"][0];
    for key in [
        "layer",
        "palette",
        "path",
        "status",
        "identical_tiles",
        "identical_pct",
        "max_channel_delta",
        "diff_kinds",
        "only_python",
        "only_rust",
    ] {
        assert!(f.get(key).is_some(), "missing frame.{key}");
    }
    assert_eq!(v["summary"]["status"], "ok");
    assert_eq!(v["mode"], "threshold");
}

/// Throughput check on a full-size pyramid: `cargo test -p radar-debug --release --test parity -- --ignored --nocapture`.
#[test]
#[ignore]
fn bench_full_pyramid() {
    let r = roots();
    let n = 1200u32;
    let idx: Vec<u8> = (0..256 * 256u32).map(|i| ((i / 7) % 4) as u8).collect();
    let rgba = to_rgba(&idx);
    for i in 0..n {
        let (z, x, y) = (7, i / 40, i % 40);
        encode(
            &tile_path(&r.py, "radar", "classic", T1, z, x, y),
            256,
            256,
            &rgba,
            false,
        );
        encode(
            &tile_path(&r.rs, "radar", "classic", T1, z, x, y),
            256,
            256,
            &idx,
            true,
        );
    }
    let t = std::time::Instant::now();
    let rep = run(&r, &opts());
    let dt = t.elapsed();
    assert_eq!(rep.summary.identical_tiles, n as usize);
    eprintln!(
        "{n} tile pairs (256x256) in {dt:?} on {} threads",
        rayon::current_num_threads()
    );
}
