//! Shared helpers for the integration tests: fixture loading and PNG tree comparison.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use radar_tiler::{ColorTable, decode_indexed, load_palette};

/// Absolute path of `testdata/<rel>`.
pub fn testdata(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join(rel)
}

/// Read a little-endian `f32` binary file.
pub fn read_f32(path: &Path) -> Vec<f32> {
    fs::read(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// Read a little-endian `f64` binary file.
pub fn read_f64(path: &Path) -> Vec<f64> {
    fs::read(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
        .collect()
}

/// Read a little-endian `i16` binary file.
pub fn read_i16(path: &Path) -> Vec<i16> {
    fs::read(path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

/// Minimal extraction of an integer or float field from the flat fixture `meta.json`.
pub fn meta_usize(meta: &serde_json::Value, key: &str) -> usize {
    meta[key]
        .as_u64()
        .unwrap_or_else(|| panic!("meta.json missing {key}")) as usize
}

/// The color table stored under `key` in shipped palette `name`.
pub fn palette_table(name: &str, key: &str) -> ColorTable {
    load_palette(name)
        .expect("shipped palette")
        .color_table(key)
        .expect("color key")
}

/// Relative `z/x/y.png` paths under `root`, sorted.
pub fn png_tree(root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    fn walk(dir: &Path, base: &Path, out: &mut BTreeSet<String>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.extension().is_some_and(|e| e == "png") {
                let rel = path.strip_prefix(base).expect("under base");
                out.insert(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    walk(root, root, &mut out);
    out
}

/// Assert that `produced` holds exactly the golden tile set and that every tile decodes to the
/// same class indices. Returns the number of tiles compared.
pub fn assert_tiles_match_golden(golden: &Path, produced: &Path) -> usize {
    let want = png_tree(golden);
    let got = png_tree(produced);
    assert!(
        !want.is_empty(),
        "no golden tiles under {}",
        golden.display()
    );
    assert_eq!(got, want, "tile sets differ for {}", golden.display());
    for rel in &want {
        let g = decode_indexed(&fs::read(golden.join(rel)).expect("golden png"))
            .expect("decode golden");
        let p = decode_indexed(&fs::read(produced.join(rel)).expect("produced png"))
            .expect("decode produced");
        assert_eq!((p.width, p.height), (g.width, g.height), "{rel}: size");
        let mismatches = p
            .indices
            .iter()
            .zip(&g.indices)
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            mismatches, 0,
            "{rel}: {mismatches} class indices differ from Python"
        );
    }
    want.len()
}
