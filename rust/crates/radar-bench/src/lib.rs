//! Measurement helpers for the Rust side of the radar-ng before/after benchmark.
//!
//! The rules match `bench/py/common.py` exactly so the two sides are comparable:
//! wall time is measured around one call; CPU time is user plus system time of the whole
//! process (`getrusage(RUSAGE_SELF)`); memory comes from `VmRSS` and `VmHWM` in
//! `/proc/self/status`. Each process runs `warmup` iterations, then `iters` timed ones.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// User plus system CPU seconds of this process, all threads included.
pub fn cpu_seconds() -> f64 {
    // SAFETY: `getrusage` only writes into the struct we pass; RUSAGE_SELF is always valid.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let secs = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 * 1e-6;
    secs(ru.ru_utime) + secs(ru.ru_stime)
}

/// `(VmRSS, VmHWM)` of this process in MB, from `/proc/self/status`.
pub fn proc_status_mb() -> (f64, f64) {
    let text = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut rss = 0.0;
    let mut hwm = 0.0;
    for line in text.lines() {
        let kb = |l: &str| {
            l.split_whitespace()
                .next()
                .and_then(|v| v.parse::<f64>().ok())
        };
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            rss = kb(rest).unwrap_or(0.0) / 1024.0;
        } else if let Some(rest) = line.strip_prefix("VmHWM:") {
            hwm = kb(rest).unwrap_or(0.0) / 1024.0;
        }
    }
    (rss, hwm)
}

/// Run `f(i)` for `i` in `0..warmup + iters` and return one JSON record per call, with the
/// same keys as the Python `measure`. `f` returns extra fields to merge into the record.
pub fn measure<F>(warmup: usize, iters: usize, mut f: F) -> Result<Vec<Value>>
where
    F: FnMut(usize) -> Result<Value>,
{
    let mut records = Vec::with_capacity(warmup + iters);
    for i in 0..warmup + iters {
        let wall0 = Instant::now();
        let cpu0 = cpu_seconds();
        let extra = f(i)?;
        let wall = wall0.elapsed().as_secs_f64();
        let cpu = cpu_seconds() - cpu0;
        let mut rec = Map::new();
        rec.insert(
            "phase".into(),
            json!(if i < warmup { "warmup" } else { "iter" }),
        );
        rec.insert("index".into(), json!(i));
        rec.insert("wall_s".into(), json!(wall));
        rec.insert("cpu_s".into(), json!(cpu));
        if let Value::Object(m) = extra {
            rec.extend(m);
        }
        records.push(Value::Object(rec));
    }
    Ok(records)
}

/// Linear-interpolation percentile (numpy default), `q` in `[0, 100]`.
pub fn percentile(values: &[f64], q: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut xs = values.to_vec();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let k = (xs.len() - 1) as f64 * q / 100.0;
    let lo = k.floor() as usize;
    let hi = (lo + 1).min(xs.len() - 1);
    xs[lo] + (xs[hi] - xs[lo]) * (k - lo as f64)
}

/// Same fields as `bench/py/common.py::summarize`.
pub fn summarize(records: &[Value]) -> Value {
    let timed: Vec<&Value> = {
        let it: Vec<&Value> = records.iter().filter(|r| r["phase"] == "iter").collect();
        if it.is_empty() {
            records.iter().collect()
        } else {
            it
        }
    };
    let walls: Vec<f64> = timed
        .iter()
        .map(|r| r["wall_s"].as_f64().unwrap_or(f64::NAN))
        .collect();
    let cpus: Vec<f64> = timed
        .iter()
        .map(|r| r["cpu_s"].as_f64().unwrap_or(f64::NAN))
        .collect();
    let cold = records
        .iter()
        .find(|r| r["phase"] == "warmup")
        .map(|r| r["wall_s"].clone());
    json!({
        "n": timed.len(),
        "wall_s_median": percentile(&walls, 50.0),
        "wall_s_p90": percentile(&walls, 90.0),
        "wall_s_min": walls.iter().copied().fold(f64::INFINITY, f64::min),
        "cpu_s_median": percentile(&cpus, 50.0),
        "cpu_s_p90": percentile(&cpus, 90.0),
        "cold_wall_s": cold,
    })
}

/// Hex SHA-256 of a byte slice.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_of(Sha256::digest(bytes).as_slice())
}

/// SHA-256 of the little-endian bytes of an `f32` slice, streamed in chunks.
pub fn sha256_f32_le(values: &[f32]) -> String {
    let mut h = Sha256::new();
    let mut buf = Vec::with_capacity(4 * 65536);
    for chunk in values.chunks(65536) {
        buf.clear();
        for v in chunk {
            buf.extend_from_slice(&v.to_le_bytes());
        }
        h.update(&buf);
    }
    hex_of(h.finalize().as_slice())
}

/// SHA-256 of the little-endian bytes of an `f64` slice.
pub fn sha256_f64_le(values: &[f64]) -> String {
    let mut h = Sha256::new();
    for v in values {
        h.update(v.to_le_bytes());
    }
    hex_of(h.finalize().as_slice())
}

/// Hex SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f =
        BufReader::new(File::open(path).with_context(|| format!("open {}", path.display()))?);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex_of(h.finalize().as_slice()))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read a raw little-endian `f32` file. The buffer is filled in chunks, so the peak is the
/// output vector alone, as with `np.fromfile`.
pub fn read_f32_le(path: &Path, expect_len: usize) -> Result<Vec<f32>> {
    let len = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len() as usize;
    if len != expect_len * 4 {
        bail!(
            "{}: {} bytes, expected {}",
            path.display(),
            len,
            expect_len * 4
        );
    }
    let mut f = File::open(path)?;
    let mut out = vec![0f32; expect_len];
    let mut buf = vec![0u8; 1 << 20];
    let mut filled = 0usize; // in f32 units
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        carry.extend_from_slice(&buf[..n]);
        let whole = carry.len() / 4 * 4;
        for c in carry[..whole].chunks_exact(4) {
            out[filled] = f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
            filled += 1;
        }
        carry.drain(..whole);
    }
    debug_assert_eq!(filled, expect_len);
    Ok(out)
}

/// Read a raw little-endian `f64` file.
pub fn read_f64_le(path: &Path) -> Result<Vec<f64>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(bytes
        .chunks_exact(8)
        .map(|c| f64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
        .collect())
}

/// Count `.png` files and their total size under `root` (recursive).
pub fn count_pngs(root: &Path) -> Result<(u64, u64)> {
    let mut n = 0u64;
    let mut bytes = 0u64;
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                stack.push(entry.path());
            } else if entry.path().extension().is_some_and(|e| e == "png") {
                n += 1;
                bytes += entry.metadata()?.len();
            }
        }
    }
    Ok((n, bytes))
}

/// CPU model, logical CPUs, total memory and kernel, read from /proc.
pub fn machine_info() -> Value {
    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let model = cpuinfo
        .lines()
        .find_map(|l| {
            l.strip_prefix("model name")
                .and_then(|r| r.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".into());
    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mem_mb = meminfo
        .lines()
        .find_map(|l| {
            l.strip_prefix("MemTotal:")
                .and_then(|r| r.split_whitespace().next()?.parse::<u64>().ok())
        })
        .map(|kb| kb / 1024);
    json!({
        "cpu_model": model,
        "logical_cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "mem_total_mb": mem_mb,
        "kernel": fs::read_to_string("/proc/sys/kernel/osrelease").unwrap_or_default().trim(),
    })
}

/// Pretty-print `value` to `path` through a temporary file and a rename.
pub fn write_json(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(value)? + "\n")?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Parse `inputs/manifest.json` written by `bench/py/gen_inputs.py`.
pub fn load_manifest(input_dir: &Path) -> Result<Value> {
    let text = fs::read_to_string(input_dir.join("manifest.json")).with_context(|| {
        format!(
            "read {}/manifest.json (run bench/py/gen_inputs.py)",
            input_dir.display()
        )
    })?;
    Ok(serde_json::from_str(&text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_matches_numpy_linear() {
        // numpy.percentile([1,2,3,4,5], [50, 90]) == [3.0, 4.6]
        let xs = [1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&xs, 50.0) - 3.0).abs() < 1e-12);
        assert!((percentile(&xs, 90.0) - 4.6).abs() < 1e-12);
        // Unsorted input gives the same answer.
        assert!((percentile(&[5.0, 1.0, 3.0, 2.0, 4.0], 90.0) - 4.6).abs() < 1e-12);
    }

    #[test]
    fn summarize_uses_timed_records_and_reports_cold_call() {
        let recs = vec![
            json!({"phase": "warmup", "wall_s": 9.0, "cpu_s": 8.0}),
            json!({"phase": "iter", "wall_s": 1.0, "cpu_s": 0.5}),
            json!({"phase": "iter", "wall_s": 3.0, "cpu_s": 1.5}),
        ];
        let s = summarize(&recs);
        assert_eq!(s["n"], 2);
        assert!((s["wall_s_median"].as_f64().unwrap() - 2.0).abs() < 1e-12);
        assert!((s["cold_wall_s"].as_f64().unwrap() - 9.0).abs() < 1e-12);
    }

    #[test]
    fn f32_digest_is_little_endian_bytes() {
        let v = [1.0f32, -2.5];
        let mut bytes = Vec::new();
        for x in v {
            bytes.extend_from_slice(&x.to_le_bytes());
        }
        assert_eq!(sha256_f32_le(&v), sha256_hex(&bytes));
    }
}
