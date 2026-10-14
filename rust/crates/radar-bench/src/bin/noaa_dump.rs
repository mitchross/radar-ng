//! Rust side of the pre-cutover NOAA check (`rust/scripts/preflight.sh`, step "noaa").
//!
//! Two subcommands:
//!
//! - `fetch`: downloads the inputs with the production `radar-grib` network code: the newest MRMS
//!   `.grib2.gz` (`mrms::list_recent_keys`), the latest HRRR `wrfsfcf01.grib2` and its `.idx`
//!   (`hrrr::find_latest_run`, whole-object GET), the 13-variable subset through `hrrr::fetch_hour`
//!   (the `.idx` plus ranged GETs that must answer 206), and optionally the newest NAQFC
//!   `ave_1hr_pm25_bc` file (`naqfc::find_latest_run`, `download_product`). It also HEADs a key that
//!   does not exist in the HRRR, NAQFC and MRMS buckets and records the status (403 or 404), and HEADs
//!   both NAQFC sources for the latest run. Files go to a cache directory and are reused.
//! - `dump`: decodes those files with `radar-grib` and writes what `bench/py/verify_noaa.py`
//!   compares against pygrib: values (f32 LE), axes and meshes (f64 LE), per-message metadata and
//!   value hashes, `.idx` record picks and selector picks for the 13 HRRR variables.
//!
//! Hashes are SHA-256 of the little-endian f32 values with every NaN written as `0x7fc00000`,
//! the same canonical form the Python side hashes.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use clap::{Parser, Subcommand};
use radar_geo::{Grid, GridGeo};
use radar_grib::decode::{MessageInfo, decode_first_message, find_message, list_messages};
use radar_grib::hrrr::{self, Fetched, HrrrVar};
use radar_grib::idx::{parse_idx, pick_ranges};
use radar_grib::mrms::{decode_gz, ensure_south_up, gunzip, list_recent_keys};
use radar_grib::naqfc::{self, AqmSources};
use radar_grib::s3::S3Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The 13 HRRR variables in the Python `IDX_MATCHERS` order.
const ALL_VARS: [HrrrVar; 13] = [
    HrrrVar::Refc,
    HrrrVar::T2m,
    HrrrVar::Dpt2m,
    HrrrVar::Cape,
    HrrrVar::U10,
    HrrrVar::V10,
    HrrrVar::Crain,
    HrrrVar::Csnow,
    HrrrVar::Cfrzr,
    HrrrVar::Cicep,
    HrrrVar::Rh2m,
    HrrrVar::Apcp,
    HrrrVar::Tcdc,
];

const MRMS_BASE: &str = "https://noaa-mrms-pds.s3.amazonaws.com";
const NAQFC_BUCKET: &str = "https://noaa-nws-naqfc-pds.s3.amazonaws.com";

#[derive(Parser)]
#[command(about = "Fetch and dump real NOAA files for the Python-vs-Rust preflight comparison")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download the newest MRMS file and the latest HRRR f01 (+ .idx, + ranged subset) into --cache.
    Fetch {
        /// Cache directory (bench/work/noaa). Files already there are reused.
        #[arg(long)]
        cache: PathBuf,
        /// MRMS product prefix in noaa-mrms-pds.
        #[arg(long, default_value = "CONUS/MergedBaseReflectivityQC_00.50")]
        mrms_prefix: String,
        /// Use the newest files already in the cache and make no request.
        #[arg(long)]
        cached_only: bool,
        /// Also fetch the newest NAQFC ave_1hr_pm25_bc file (README items 9 and 10).
        #[arg(long)]
        naqfc: bool,
        /// Where to write the fetch report.
        #[arg(long)]
        json: PathBuf,
    },
    /// Decode the files and write values, axes and selections for verify_noaa.py.
    Dump {
        /// MRMS `.grib2.gz` (or a raw `.grib2`).
        #[arg(long)]
        mrms: Option<PathBuf>,
        /// HRRR `wrfsfcf01.grib2` (whole file).
        #[arg(long)]
        hrrr: Option<PathBuf>,
        /// HRRR `.idx` for --hrrr.
        #[arg(long)]
        hrrr_idx: Option<PathBuf>,
        /// The ranged subset fetched by `hrrr::fetch_hour` (optional).
        #[arg(long)]
        hrrr_subset: Option<PathBuf>,
        /// NAQFC GRIB2 file (optional, first message only).
        #[arg(long)]
        naqfc: Option<PathBuf>,
        /// Output directory.
        #[arg(long)]
        out: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Fetch {
            cache,
            mrms_prefix,
            cached_only,
            naqfc,
            json,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            let report = rt.block_on(fetch(&cache, &mrms_prefix, cached_only, naqfc))?;
            radar_bench::write_json(&json, &report)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Cmd::Dump {
            mrms,
            hrrr,
            hrrr_idx,
            hrrr_subset,
            naqfc,
            out,
        } => {
            fs::create_dir_all(&out)?;
            let mut report = json!({});
            if let Some(path) = mrms {
                report["mrms"] = dump_mrms(&path, &out)?;
            }
            if let Some(path) = hrrr {
                report["hrrr"] =
                    dump_hrrr(&path, hrrr_idx.as_deref(), hrrr_subset.as_deref(), &out)?;
            }
            if let Some(path) = naqfc {
                report["naqfc"] = dump_naqfc(&path, &out)?;
            }
            radar_bench::write_json(&out.join("rust.json"), &report)?;
            println!("rust dump written to {}", out.join("rust.json").display());
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- fetch

/// Writes `bytes` to `path` through a `.part` file, so an existing file is always complete.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let part = path.with_extension("part");
    fs::write(&part, bytes).with_context(|| format!("write {}", part.display()))?;
    fs::rename(&part, path)?;
    Ok(())
}

/// Newest entry (by name) in `dir` whose file name starts with `prefix` and ends with `suffix`.
/// Names sort by time for every cache entry this tool writes (keys and run ids carry the time).
fn newest_cached(dir: &Path, prefix: &str, suffix: &str) -> Option<PathBuf> {
    let mut names: Vec<PathBuf> = fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            name.starts_with(prefix) && name.ends_with(suffix)
        })
        .collect();
    names.sort();
    names.pop()
}

async fn fetch(
    cache: &Path,
    mrms_prefix: &str,
    cached_only: bool,
    with_naqfc: bool,
) -> Result<Value> {
    let http = radar_grib::http::client()?;
    let now = Utc::now();
    let mut report = json!({ "now": now.to_rfc3339(), "cached_only": cached_only });

    // MRMS: newest key for today (yesterday when today is empty), via the production listing.
    let mrms_dir = cache.join("mrms");
    let product = mrms_prefix.rsplit('/').next().unwrap_or(mrms_prefix);
    if cached_only {
        let file = newest_cached(&mrms_dir, &format!("MRMS_{product}_"), ".grib2.gz")
            .with_context(|| format!("no cached MRMS file in {}", mrms_dir.display()))?;
        report["mrms"] = json!({ "file": file, "reused": true });
    } else {
        let s3 = S3Client::new(http.clone(), MRMS_BASE);
        let keys = list_recent_keys(&s3, mrms_prefix, now)
            .await
            .with_context(|| format!("list {MRMS_BASE}/{mrms_prefix}"))?;
        let key = keys
            .last()
            .with_context(|| format!("no MRMS keys under {mrms_prefix}"))?;
        let file = mrms_dir.join(key.rsplit('/').next().unwrap_or(key));
        let reused = file.exists();
        let valid_time = radar_grib::mrms::extract_timestamp(key)?;
        if !reused {
            let (_, bytes) = radar_grib::mrms::fetch_gz(&s3, key).await?;
            write_atomic(&file, &bytes)?;
        }
        report["mrms"] = json!({
            "prefix": mrms_prefix, "product": product, "listed_keys": keys.len(),
            "key": key, "valid_time": valid_time, "file": file, "reused": reused,
        });
    }

    // HRRR: latest run whose f01 answers HEAD 200; whole file, .idx, and the fetch_hour subset.
    let hrrr_root = cache.join("hrrr");
    if cached_only {
        let run_dir = newest_cached(&hrrr_root, "", "")
            .filter(|p| p.is_dir())
            .with_context(|| format!("no cached HRRR run in {}", hrrr_root.display()))?;
        let file = newest_cached(&run_dir, "hrrr.", ".grib2").context("no cached HRRR .grib2")?;
        let idx = PathBuf::from(format!("{}.idx", file.display()));
        let subset = run_dir.join("subset_f01.grib2");
        report["hrrr"] = json!({
            "file": file, "idx_file": idx,
            "subset_file": if subset.exists() { json!(subset) } else { Value::Null },
            "reused": true,
        });
    } else {
        let s3 = S3Client::new(http.clone(), hrrr::BASE_URL);
        let run = hrrr::find_latest_run(&s3, now)
            .await?
            .context("no HRRR run with an f01 answering HEAD 200 in the last 12 hours")?;
        let init = hrrr::parse_run_id(&run).context("bad run id")?;
        let key = hrrr::file_key(init, 1);
        let run_dir = hrrr_root.join(&run);
        let file = run_dir.join(key.rsplit('/').next().unwrap_or(&key));
        let idx = PathBuf::from(format!("{}.idx", file.display()));
        let subset = run_dir.join("subset_f01.grib2");
        let reused = file.exists() && idx.exists();
        if !idx.exists() {
            let bytes = s3
                .get_object(&format!("{key}.idx"), radar_grib::http::IDX_TIMEOUT)
                .await?;
            write_atomic(&idx, &bytes)?;
        }
        if !file.exists() {
            let bytes = s3
                .get_object(&key, radar_grib::http::FULL_FILE_TIMEOUT)
                .await?;
            write_atomic(&file, &bytes)?;
        }
        // README item 12: ranged GETs must answer 206 with a matching Content-Range.
        let subset_result = match hrrr::fetch_hour(&s3, init, 1, &ALL_VARS).await {
            Ok(Fetched::Data { bytes, full_file }) => {
                write_atomic(&subset, &bytes)?;
                json!({ "ok": true, "full_file": full_file, "bytes": bytes.len() })
            }
            Ok(other) => json!({ "ok": false, "error": format!("{other:?}") }),
            Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
        };
        report["hrrr"] = json!({
            "run": run, "key": key, "file": file, "idx_file": idx,
            "subset_file": if subset.exists() { json!(subset) } else { Value::Null },
            "fetch_hour": subset_result, "reused": reused,
        });
    }

    if cached_only {
        report["missing_object_status"] = Value::Null;
    } else {
        // README item 1: what anonymous S3 answers for a key that does not exist.
        let stamp = now.format("%Y%m%d%H%M%S");
        let mut probes = serde_json::Map::new();
        for (name, base, key) in [
            (
                "noaa-hrrr-bdp-pds",
                hrrr::BASE_URL.to_string(),
                format!("hrrr.19700101/conus/radar-ng-preflight-missing-{stamp}.grib2"),
            ),
            (
                "noaa-nws-naqfc-pds",
                NAQFC_BUCKET.to_string(),
                format!("AQMv7/CS/19700101/12/radar-ng-preflight-missing-{stamp}.grib2"),
            ),
            (
                "noaa-mrms-pds",
                MRMS_BASE.to_string(),
                format!("CONUS/radar-ng-preflight-missing-{stamp}.grib2.gz"),
            ),
        ] {
            let s3 = S3Client::new(http.clone(), base.clone());
            let head = s3.head_status(&key, radar_grib::http::HEAD_TIMEOUT).await;
            let get = s3.get_object(&key, radar_grib::http::IDX_TIMEOUT).await;
            probes.insert(
                name.to_string(),
                json!({
                    "url": format!("{base}/{key}"),
                    "head_status": head.as_ref().ok(),
                    "head_error": head.as_ref().err().map(|e| format!("{e:#}")),
                    "get_error": get.as_ref().err().map(|e| format!("{e:#}")),
                    "get_is_not_found": get.as_ref().err().map(|e| e.is_not_found()),
                }),
            );
        }
        report["missing_object_status"] = Value::Object(probes);
    }

    if with_naqfc {
        report["naqfc"] = fetch_naqfc(&http, cache, now, cached_only).await;
    }
    Ok(report)
}

/// README items 9 and 10. Errors are recorded, not raised: this part is informational.
async fn fetch_naqfc(
    http: &reqwest::Client,
    cache: &Path,
    now: chrono::DateTime<Utc>,
    cached_only: bool,
) -> Value {
    let dir = cache.join("naqfc");
    if cached_only {
        return match newest_cached(&dir, "", ".grib2") {
            Some(file) => json!({ "file": file, "reused": true }),
            None => json!({ "error": "no cached NAQFC file" }),
        };
    }
    let sources = AqmSources::default();
    let run = match naqfc::find_latest_run(http, &sources, now).await {
        Ok(Some(run)) => run,
        Ok(None) => return json!({ "error": "no AQM run answered HEAD 200 on AWS or NOMADS" }),
        Err(e) => return json!({ "error": format!("{e:#}") }),
    };
    let mut heads = Vec::new();
    if let Ok(urls) = sources.grib_urls(&run, naqfc::PROBE_PRODUCT) {
        for url in urls {
            let entry = match http
                .head(&url)
                .timeout(radar_grib::http::HEAD_TIMEOUT)
                .send()
                .await
            {
                Ok(resp) => json!({
                    "url": url, "status": resp.status().as_u16(),
                    "location": resp.headers().get("location").and_then(|v| v.to_str().ok()),
                }),
                Err(e) => json!({ "url": url, "error": e.to_string() }),
            };
            heads.push(entry);
        }
    }
    let file = dir.join(format!("{run}.{}.227.grib2", naqfc::PROBE_PRODUCT));
    let reused = file.exists();
    let download = if reused {
        json!({ "ok": true })
    } else {
        match naqfc::download_product(http, &sources, &run, naqfc::PROBE_PRODUCT).await {
            Ok(bytes) => match write_atomic(&file, &bytes) {
                Ok(()) => json!({ "ok": true, "bytes": bytes.len() }),
                Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
            },
            Err(e) => json!({ "ok": false, "error": format!("{e:#}") }),
        }
    };
    json!({
        "run": run, "heads": heads, "download": download, "reused": reused,
        "file": if file.exists() { json!(file) } else { Value::Null },
    })
}

// ---------------------------------------------------------------- dump

/// SHA-256 of the values as little-endian f32, every NaN written as 0x7fc00000.
fn canonical_hash(values: &[f32]) -> String {
    let mut h = Sha256::new();
    let mut buf = Vec::with_capacity(4 * 65536);
    for chunk in values.chunks(65536) {
        buf.clear();
        for v in chunk {
            let bits = if v.is_nan() {
                0x7fc0_0000u32
            } else {
                v.to_bits()
            };
            buf.extend_from_slice(&bits.to_le_bytes());
        }
        h.update(&buf);
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn write_f32(path: &Path, values: &[f32]) -> Result<()> {
    let mut buf = Vec::with_capacity(values.len() * 4);
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(path, buf).with_context(|| format!("write {}", path.display()))
}

fn write_f64(path: &Path, values: &[f64]) -> Result<()> {
    let mut buf = Vec::with_capacity(values.len() * 8);
    for v in values {
        buf.extend_from_slice(&v.to_le_bytes());
    }
    fs::write(path, buf).with_context(|| format!("write {}", path.display()))
}

fn info_json(index: usize, info: &MessageInfo) -> Value {
    json!({
        "index": index,
        "discipline": info.discipline,
        "category": info.category,
        "number": info.number,
        "surface_type": info.surface_type,
        "level_value": info.level_value,
        "grid_template": info.grid_template,
        "data_template": info.data_template,
        "resolution_flags": info.resolution_flags,
    })
}

fn dump_mrms(path: &Path, out: &Path) -> Result<Value> {
    let raw = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let gz = raw.starts_with(&[0x1f, 0x8b]);
    let plain = if gz { gunzip(&raw)? } else { raw.clone() };
    let infos = list_messages(&plain)?;
    // The production path: decode_gz (first message), then the caller's ensure_south_up.
    let mut grid = if gz {
        decode_gz(&raw)?
    } else {
        radar_grib::decode::decode_message(&raw)?
    };
    let GridGeo::Regular { lats, .. } = &grid.geo else {
        bail!("MRMS: expected a regular lat/lon grid");
    };
    let stored = (lats[0], lats[lats.len() - 1]);
    let flipped = ensure_south_up(&mut grid);
    let GridGeo::Regular { lats, lons } = &grid.geo else {
        unreachable!()
    };
    write_f32(&out.join("mrms_values.f32le"), &grid.values)?;
    write_f64(&out.join("mrms_lats.f64le"), lats)?;
    write_f64(&out.join("mrms_lons.f64le"), lons)?;
    Ok(json!({
        "file": path,
        "messages": infos.iter().enumerate().map(|(i, m)| info_json(i + 1, m)).collect::<Vec<_>>(),
        "shape": [grid.height, grid.width],
        "stored_lat_first": stored.0, "stored_lat_last": stored.1,
        "flipped": flipped,
        "lat_first": lats[0], "lat_last": lats[lats.len() - 1],
        "lon_first": lons[0], "lon_last": lons[lons.len() - 1],
        "nan_cells": grid.values.iter().filter(|v| v.is_nan()).count(),
        "values_sha256": canonical_hash(&grid.values),
        "values_file": "mrms_values.f32le", "lats_file": "mrms_lats.f64le", "lons_file": "mrms_lons.f64le",
    }))
}

/// Splits a GRIB2 stream into messages by the 8-byte total length (octets 9 to 16).
fn split_messages(bytes: &[u8]) -> Result<Vec<&[u8]>> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(pos) = bytes[at..].windows(4).position(|w| w == b"GRIB") else {
            break;
        };
        let start = at + pos;
        if start + 16 > bytes.len() {
            bail!("truncated GRIB header at byte {start}");
        }
        let len = u64::from_be_bytes(bytes[start + 8..start + 16].try_into()?) as usize;
        if len < 16 || start + len > bytes.len() {
            bail!("bad GRIB length {len} at byte {start}");
        }
        out.push(&bytes[start..start + len]);
        at = start + len;
    }
    Ok(out)
}

fn mesh_of(grid: &Grid) -> Option<(&[f64], &[f64])> {
    match &grid.geo {
        GridGeo::Projected {
            lat_mesh, lon_mesh, ..
        } => Some((lat_mesh, lon_mesh)),
        GridGeo::Regular { .. } => None,
    }
}

fn mesh_hash(grid: &Grid) -> Option<String> {
    let (lat, lon) = mesh_of(grid)?;
    let mut h = Sha256::new();
    for v in lat.iter().chain(lon) {
        h.update(v.to_le_bytes());
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn dump_hrrr(path: &Path, idx: Option<&Path>, subset: Option<&Path>, out: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let infos = list_messages(&bytes)?;
    let chunks = split_messages(&bytes)?;

    // README item 7: decode every message of the file, not only the 13 the pipeline reads.
    let mut per_message = Vec::with_capacity(chunks.len());
    for (i, chunk) in chunks.iter().enumerate() {
        let entry = match decode_first_message(chunk) {
            Ok(msg) => json!({
                "index": i + 1, "ok": true,
                "data_template": msg.info.data_template,
                "values_sha256": canonical_hash(&msg.grid.values),
                "nan_cells": msg.grid.values.iter().filter(|v| v.is_nan()).count(),
            }),
            Err(e) => json!({ "index": i + 1, "ok": false, "error": format!("{e:#}") }),
        };
        per_message.push(entry);
    }

    // README item 2: .idx records and the Rust picks per variable (record numbers).
    let mut idx_report = Value::Null;
    if let Some(idx_path) = idx {
        let text =
            fs::read_to_string(idx_path).with_context(|| format!("read {}", idx_path.display()))?;
        let records = parse_idx(&text)?;
        let mut picks = serde_json::Map::new();
        for var in ALL_VARS {
            let ranges = pick_ranges(&records, &[var.idx_matcher()]);
            let nums: Vec<u64> = ranges
                .iter()
                .filter_map(|r| {
                    records
                        .iter()
                        .find(|rec| rec.offset == r.start)
                        .map(|rec| rec.num)
                })
                .collect();
            let ranges_json: Vec<Value> = ranges
                .iter()
                .map(|r| json!({ "start": r.start, "end": r.end }))
                .collect();
            picks.insert(
                var.key().to_string(),
                json!({ "records": nums, "ranges": ranges_json }),
            );
        }
        idx_report = json!({ "file": idx_path, "records": records.len(), "picks": picks });
    }

    // README items 3, 4 and 6: selector picks, values and the Lambert lat/lon mesh.
    let mut vars = serde_json::Map::new();
    let mut mesh_written: Option<String> = None;
    for var in ALL_VARS {
        let sel = var.selector();
        let matching: Vec<usize> = infos
            .iter()
            .enumerate()
            .filter(|(_, m)| sel.matches(m))
            .map(|(i, _)| i + 1)
            .collect();
        let entry = match find_message(&bytes, &sel) {
            Ok(Some(msg)) => {
                let file = format!("hrrr_{}_values.f32le", var.key());
                write_f32(&out.join(&file), &msg.grid.values)?;
                let mh = mesh_hash(&msg.grid);
                if mesh_written.is_none()
                    && let Some((lat, lon)) = mesh_of(&msg.grid)
                {
                    write_f64(&out.join("hrrr_lats.f64le"), lat)?;
                    write_f64(&out.join("hrrr_lons.f64le"), lon)?;
                    mesh_written = mh.clone();
                }
                json!({
                    "selected": matching.first(), "matching": matching,
                    "info": info_json(matching.first().copied().unwrap_or(0), &msg.info),
                    "shape": [msg.grid.height, msg.grid.width],
                    "values_file": file, "values_sha256": canonical_hash(&msg.grid.values),
                    "nan_cells": msg.grid.values.iter().filter(|v| v.is_nan()).count(),
                    "uv_grid_relative": msg.uv_grid_relative(),
                    "mesh_same_as_written": mh.is_some() && mh == mesh_written,
                })
            }
            Ok(None) => json!({ "selected": Value::Null, "matching": matching }),
            Err(e) => {
                json!({ "selected": matching.first(), "matching": matching, "error": format!("{e:#}") })
            }
        };
        vars.insert(var.key().to_string(), entry);
    }

    // The fetch_hour subset must decode to the same values as the whole file.
    let mut subset_report = Value::Null;
    if let Some(sub_path) = subset {
        let sub = fs::read(sub_path).with_context(|| format!("read {}", sub_path.display()))?;
        let mut per_var = serde_json::Map::new();
        for var in ALL_VARS {
            let v = match find_message(&sub, &var.selector()) {
                Ok(Some(msg)) => json!({ "values_sha256": canonical_hash(&msg.grid.values) }),
                Ok(None) => json!({ "missing": true }),
                Err(e) => json!({ "error": format!("{e:#}") }),
            };
            per_var.insert(var.key().to_string(), v);
        }
        subset_report = json!({
            "file": sub_path, "messages": list_messages(&sub).map(|m| m.len()).ok(), "vars": per_var,
        });
    }

    Ok(json!({
        "file": path,
        "message_count_listed": infos.len(),
        "message_count_split": chunks.len(),
        "messages": infos.iter().enumerate().map(|(i, m)| info_json(i + 1, m)).collect::<Vec<_>>(),
        "decode": per_message,
        "idx": idx_report,
        "vars": vars,
        "mesh_files": if mesh_written.is_some() { json!(["hrrr_lats.f64le", "hrrr_lons.f64le"]) } else { Value::Null },
        "subset": subset_report,
    }))
}

fn dump_naqfc(path: &Path, out: &Path) -> Result<Value> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let infos = list_messages(&bytes);
    let first = decode_first_message(&bytes);
    let mut report = json!({
        "file": path,
        "message_count": infos.as_ref().map(|m| m.len()).ok(),
        "first_info": infos.as_ref().ok().and_then(|m| m.first()).map(|m| info_json(1, m)),
    });
    match first {
        Ok(msg) => {
            write_f32(&out.join("naqfc_values.f32le"), &msg.grid.values)?;
            report["values_sha256"] = json!(canonical_hash(&msg.grid.values));
            report["shape"] = json!([msg.grid.height, msg.grid.width]);
            match &msg.grid.geo {
                GridGeo::Projected {
                    x,
                    y,
                    lat_mesh,
                    lon_mesh,
                    ..
                } => {
                    write_f64(&out.join("naqfc_lats.f64le"), lat_mesh)?;
                    write_f64(&out.join("naqfc_lons.f64le"), lon_mesh)?;
                    write_f64(&out.join("naqfc_x.f64le"), x)?;
                    write_f64(&out.join("naqfc_y.f64le"), y)?;
                    report["geo"] = json!("projected");
                }
                GridGeo::Regular { lats, lons } => {
                    write_f64(&out.join("naqfc_lat_axis.f64le"), lats)?;
                    write_f64(&out.join("naqfc_lon_axis.f64le"), lons)?;
                    report["geo"] = json!("regular");
                }
            }
        }
        Err(e) => report["error"] = json!(format!("{e:#}")),
    }
    Ok(report)
}
