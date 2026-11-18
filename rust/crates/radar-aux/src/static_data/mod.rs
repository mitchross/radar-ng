//! Static reference data built once on the aux role (design `weather-extras.md` section 6).
//!
//! - [`normals`]: the 1991-2020 daily temperature normals under `STATE_DIR/static/normals/`.
//! - [`hdf5`]: the minimal pure-Rust HDF5 reader that cuts them out of the NCEI NetCDF-4 files.
//!
//! [`ensure_static_data`] is the `static_data_ensure` activity body. A present marker means done:
//! the daily schedule then costs one `stat`.

pub mod hdf5;
pub mod normals;

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use radar_types::payloads::static_data::StaticDataResult;
use serde_json::json;

use crate::beat::Beat;
use hdf5::ReadAt;
use normals::{GridSpec, Normals, NormalsMeta, Var};

/// Product name of the temperature normals.
pub const PRODUCT_NORMALS: &str = "normals";

/// Errors from building static data.
#[derive(Debug, thiserror::Error)]
pub enum StaticDataError {
    /// The workflow asked for a product this build does not know.
    #[error("unknown static-data product {0:?}")]
    UnknownProduct(String),
    /// Download, decode or write failed. Retrying may help.
    #[error("building {0}: {1}")]
    Io(&'static str, #[source] io::Error),
    /// The blocking task panicked or was cancelled.
    #[error("static-data task failed: {0}")]
    Join(String),
}

/// Static-data configuration.
#[derive(Debug, Clone)]
pub struct StaticDataConfig {
    /// `STATE_DIR`.
    pub state_dir: PathBuf,
    /// NCEI normals directory (overridable for tests and mirrors: `NORMALS_BASE_URL`).
    pub normals_base_url: String,
}

impl StaticDataConfig {
    /// Reads `STATE_DIR` and `NORMALS_BASE_URL`.
    pub fn from_env() -> Self {
        Self {
            state_dir: radar_core::env::path_var("STATE_DIR", "/data/state"),
            normals_base_url: std::env::var("NORMALS_BASE_URL")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| normals::SOURCE_BASE_URL.to_string()),
        }
    }
}

/// HTTP range reads, run from a blocking thread. Small reads fetch a 64 KiB block and keep it.
pub struct HttpRangeReader {
    client: reqwest::Client,
    url: String,
    handle: tokio::runtime::Handle,
    cache: Option<(u64, Vec<u8>)>,
}

/// Block size of the metadata reads.
const META_BLOCK: usize = 64 * 1024;

impl HttpRangeReader {
    /// A reader over `url` that runs its requests on `handle`.
    pub fn new(client: reqwest::Client, url: String, handle: tokio::runtime::Handle) -> Self {
        Self {
            client,
            url,
            handle,
            cache: None,
        }
    }

    fn fetch(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let range = format!("bytes={}-{}", offset, offset + len as u64 - 1);
        let (client, url) = (self.client.clone(), self.url.clone());
        self.handle.block_on(async move {
            let resp = client
                .get(&url)
                .header(reqwest::header::RANGE, range)
                .timeout(Duration::from_secs(120))
                .send()
                .await
                .map_err(io::Error::other)?;
            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                return Err(io::Error::other(format!("{url}: HTTP {}", resp.status())));
            }
            Ok(resp.bytes().await.map_err(io::Error::other)?.to_vec())
        })
    }
}

impl ReadAt for HttpRangeReader {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        if let Some((start, buf)) = &self.cache
            && offset >= *start
            && offset + len as u64 <= *start + buf.len() as u64
        {
            let i = (offset - start) as usize;
            return Ok(buf[i..i + len].to_vec());
        }
        if len > META_BLOCK / 2 {
            let b = self.fetch(offset, len)?;
            return exact(b, len);
        }
        let block = self.fetch(offset, META_BLOCK)?;
        if block.len() < len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short range read",
            ));
        }
        let out = block[..len].to_vec();
        self.cache = Some((offset, block));
        Ok(out)
    }
}

fn exact(b: Vec<u8>, len: usize) -> io::Result<Vec<u8>> {
    if b.len() == len {
        Ok(b)
    } else {
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("range read returned {} of {len} bytes", b.len()),
        ))
    }
}

/// Build the normals store from `open(var)` readers. Arrays already on disk at full size are kept, so
/// a retry after a failed second file does not download the first again.
pub fn build_normals(
    state_dir: &std::path::Path,
    sources: &[String],
    open: &mut dyn FnMut(Var) -> io::Result<Box<dyn ReadAt>>,
    beat: &Beat,
) -> io::Result<NormalsMeta> {
    let dir = normals::store_dir(state_dir);
    let mut grid: Option<GridSpec> = None;
    let mut bytes = 0u64;
    for var in Var::ALL {
        let mut r = open(var)?;
        let g = normals::read_grid(r.as_mut())?;
        if grid.is_some_and(|prev| prev != g) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "tmax and tmin grids differ",
            ));
        }
        grid = Some(g);
        let out = dir.join(var.file());
        if std::fs::metadata(&out).is_ok_and(|m| m.len() == g.file_bytes()) {
            bytes += g.file_bytes();
            continue;
        }
        let name = var.name();
        bytes += normals::convert(r.as_mut(), var, &g, &out, &mut |day| {
            if day % 10 == 0 {
                beat(json!({"phase": "convert", "var": name, "day": day}));
            }
        })?;
    }
    let meta = NormalsMeta {
        version: normals::FORMAT_VERSION,
        grid: grid.expect("two arrays"),
        days: normals::DAYS,
        scale: 0.1,
        period: "1991-2020".into(),
        sources: sources.to_vec(),
        bytes,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    normals::write_meta(state_dir, &meta)?;
    Ok(meta)
}

/// `static_data_ensure`: build `product` if its marker is missing, else do nothing.
pub async fn ensure_static_data(
    config: &StaticDataConfig,
    client: &reqwest::Client,
    product: &str,
    beat: Beat,
) -> Result<StaticDataResult, StaticDataError> {
    if product != PRODUCT_NORMALS {
        return Err(StaticDataError::UnknownProduct(product.to_owned()));
    }
    let io_err = |e| StaticDataError::Io(PRODUCT_NORMALS, e);
    if let Some(n) = Normals::open(&config.state_dir).map_err(io_err)? {
        return Ok(StaticDataResult {
            product: product.into(),
            outcome: "present".into(),
            bytes: Some(n.meta.bytes as i64),
        });
    }
    let (state_dir, client) = (config.state_dir.clone(), client.clone());
    let base = config.normals_base_url.clone();
    let handle = tokio::runtime::Handle::current();
    let meta = tokio::task::spawn_blocking(move || {
        let sources: Vec<String> = Var::ALL.iter().map(|v| v.source_url(&base)).collect();
        let mut open = |var: Var| -> io::Result<Box<dyn ReadAt>> {
            Ok(Box::new(HttpRangeReader::new(
                client.clone(),
                var.source_url(&base),
                handle.clone(),
            )))
        };
        build_normals(&state_dir, &sources, &mut open, &beat)
    })
    .await
    .map_err(|e| StaticDataError::Join(e.to_string()))?
    .map_err(io_err)?;
    tracing::info!(bytes = meta.bytes, "normals_built");
    Ok(StaticDataResult {
        product: product.into(),
        outcome: "built".into(),
        bytes: Some(meta.bytes as i64),
    })
}
