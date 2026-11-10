use sha2::{Digest, Sha256};
use std::{fs, path::Path};

fn collect(path: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "rs" || ext == "json")
        {
            files.push(path);
        }
    }
}

fn main() {
    let mut files = vec![
        Path::new("../../Cargo.lock").to_path_buf(),
        Path::new("build.rs").to_path_buf(),
    ];
    for name in [
        "radar-core",
        "radar-replay",
        "radar-tiler",
        "radar-grib",
        "radar-geo",
        "radar-ingest",
    ] {
        collect(&Path::new("..").join(name).join("src"), &mut files);
    }
    collect(Path::new("../radar-tiler/assets"), &mut files);
    files.sort();
    let mut digest = Sha256::new();
    for file in files {
        println!("cargo:rerun-if-changed={}", file.display());
        let bytes = fs::read(&file).unwrap();
        digest.update(file.to_string_lossy().as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    println!(
        "cargo:rustc-env=REPLAY_RENDERER_SOURCE_SHA256={:x}",
        digest.finalize()
    );
}
