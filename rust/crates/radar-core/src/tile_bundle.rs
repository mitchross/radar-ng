//! Compact binary Radar Tile Bundle (`RTB1`) format for RFC #389 Phase 3.
//!
//! Layout (all integers little-endian):
//! - Header (12 bytes): `magic = b"RTB1"`, `version = 1u32`, `count = N: u32`.
//! - Index (`N * 20` bytes, sorted ascending by `(z, x, y)`):
//!   `(z: u32, x: u32, y: u32, offset: u32, length: u32)`.
//! - Body: concatenated raw PNG bytes.
//!
//! One S3 `PUT` publishes an entire pyramid (instead of ~630 individual PNG PUTs), one S3 `GET`
//! loads it into `radar-api`'s byte-weighted `moka` cache, and individual tile lookups are O(log N)
//! binary search with zero-copy `Bytes::slice`.

use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Result, bail, ensure};
use bytes::Bytes;

/// 4-byte magic identifying a Radar Tile Bundle v1.
pub const MAGIC: &[u8; 4] = b"RTB1";
/// Supported bundle format version.
pub const VERSION: u32 = 1;
/// Header size in bytes (`magic + version + count`).
pub const HEADER_LEN: usize = 12;
/// Index entry size in bytes (`z, x, y, offset, length`).
pub const ENTRY_LEN: usize = 20;

/// Validated, zero-copy tile bundle backed by [`Bytes`].
#[derive(Debug, Clone)]
pub struct TileBundle {
    bytes: Bytes,
    count: usize,
}

impl TileBundle {
    /// Encode `(z, x, y, png_bytes)` entries into an `RTB1` byte buffer.
    ///
    /// Entries are sorted by `(z, x, y)` before writing so [`Self::get`] can binary-search.
    /// Duplicate coordinates keep the last entry in input order.
    pub fn encode(tiles: &[(u32, u32, u32, Vec<u8>)]) -> Result<Vec<u8>> {
        let mut sorted: Vec<(u32, u32, u32, &[u8])> = tiles
            .iter()
            .map(|(z, x, y, b)| (*z, *x, *y, b.as_slice()))
            .collect();
        sorted.sort_by_key(|(z, x, y, _)| (*z, *x, *y));
        // Deduplicate by coordinate (keep last).
        let mut deduped: Vec<(u32, u32, u32, &[u8])> = Vec::with_capacity(sorted.len());
        for item in sorted {
            if let Some(last) = deduped.last_mut()
                && (last.0, last.1, last.2) == (item.0, item.1, item.2)
            {
                *last = item;
            } else {
                deduped.push(item);
            }
        }

        let count = u32::try_from(deduped.len()).map_err(|_| anyhow::anyhow!("too many tiles"))?;
        let index_bytes = deduped
            .len()
            .checked_mul(ENTRY_LEN)
            .ok_or_else(|| anyhow::anyhow!("tile index size overflow"))?;
        let body_start = HEADER_LEN
            .checked_add(index_bytes)
            .ok_or_else(|| anyhow::anyhow!("tile bundle header overflow"))?;
        let total_body: usize = deduped.iter().map(|(_, _, _, b)| b.len()).sum();
        let total_size = body_start
            .checked_add(total_body)
            .ok_or_else(|| anyhow::anyhow!("tile bundle size overflow"))?;
        ensure!(
            u32::try_from(total_size).is_ok(),
            "tile bundle exceeds 4 GiB limit ({total_size} bytes)"
        );

        let mut out = Vec::with_capacity(total_size);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());

        let mut cursor = body_start as u32;
        for &(z, x, y, png) in &deduped {
            let len = png.len() as u32;
            out.extend_from_slice(&z.to_le_bytes());
            out.extend_from_slice(&x.to_le_bytes());
            out.extend_from_slice(&y.to_le_bytes());
            out.extend_from_slice(&cursor.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
            cursor += len;
        }
        for (_, _, _, png) in &deduped {
            out.extend_from_slice(png);
        }
        Ok(out)
    }

    /// Walk `<dir>/<z>/<x>/<y>.png` and encode all PNG tiles into an `RTB1` buffer.
    pub fn from_dir(dir: &Path) -> Result<Vec<u8>> {
        let mut tiles: Vec<(u32, u32, u32, Vec<u8>)> = Vec::new();
        for (z, z_dir) in numeric_children(dir, "")? {
            for (x, x_dir) in numeric_children(&z_dir, "")? {
                for (y, y_path) in numeric_children(&x_dir, ".png")? {
                    let png = fs::read(&y_path)?;
                    tiles.push((z, x, y, png));
                }
            }
        }
        Self::encode(&tiles)
    }

    /// Unpack every tile in this bundle into `<dir>/<z>/<x>/<y>.png`.
    pub fn unpack_to_dir(&self, dir: &Path) -> io::Result<()> {
        for idx in 0..self.count {
            let (z, x, y, off, len) = self.entry_at(idx);
            let tile_dir = dir.join(z.to_string()).join(x.to_string());
            fs::create_dir_all(&tile_dir)?;
            fs::write(
                tile_dir.join(format!("{y}.png")),
                &self.bytes[off..off + len],
            )?;
        }
        Ok(())
    }

    /// Validate and wrap an owned `Vec<u8>` without copying tile payloads.
    pub fn parse_vec(bytes: Vec<u8>) -> Result<Self> {
        Self::parse(Bytes::from(bytes))
    }

    /// Validate and wrap `bytes` without copying tile payloads.
    pub fn parse(bytes: Bytes) -> Result<Self> {
        ensure!(bytes.len() >= HEADER_LEN, "tile bundle truncated header");
        if &bytes[0..4] != MAGIC {
            bail!("invalid tile bundle magic");
        }
        let version = read_u32(&bytes, 4);
        ensure!(
            version == VERSION,
            "unsupported tile bundle version {version}"
        );
        let count = read_u32(&bytes, 8) as usize;
        let index_end = HEADER_LEN
            .checked_add(
                count
                    .checked_mul(ENTRY_LEN)
                    .ok_or_else(|| anyhow::anyhow!("tile bundle index overflow"))?,
            )
            .ok_or_else(|| anyhow::anyhow!("tile bundle index end overflow"))?;
        ensure!(
            bytes.len() >= index_end,
            "tile bundle truncated index (len={}, need>={index_end})",
            bytes.len()
        );
        let mut prev: Option<(u32, u32, u32)> = None;
        for idx in 0..count {
            let base = HEADER_LEN + idx * ENTRY_LEN;
            let coord = (
                read_u32(&bytes, base),
                read_u32(&bytes, base + 4),
                read_u32(&bytes, base + 8),
            );
            if let Some(p) = prev {
                ensure!(coord > p, "tile bundle index is not strictly sorted");
            }
            prev = Some(coord);
            let off = read_u32(&bytes, base + 12) as usize;
            let len = read_u32(&bytes, base + 16) as usize;
            let end = off
                .checked_add(len)
                .ok_or_else(|| anyhow::anyhow!("tile bundle slice overflow"))?;
            ensure!(
                off >= index_end && end <= bytes.len(),
                "tile bundle entry {idx} out of bounds ({off}..{end}, total={})",
                bytes.len()
            );
        }
        Ok(Self { bytes, count })
    }

    /// Number of tiles in this bundle.
    #[must_use]
    pub fn tile_count(&self) -> usize {
        self.count
    }

    /// Total encoded byte size (used as `moka` cache weight).
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    /// Look up `(z, x, y)` in O(log N) time, returning a zero-copy [`Bytes`] slice.
    #[must_use]
    pub fn get(&self, z: u32, x: u32, y: u32) -> Option<Bytes> {
        let idx = self.find_index(z, x, y)?;
        let (_, _, _, off, len) = self.entry_at(idx);
        Some(self.bytes.slice(off..off + len))
    }

    /// True if `(z, x, y)` is present in this bundle.
    #[must_use]
    pub fn contains(&self, z: u32, x: u32, y: u32) -> bool {
        self.find_index(z, x, y).is_some()
    }

    fn find_index(&self, z: u32, x: u32, y: u32) -> Option<usize> {
        let target = (z, x, y);
        let mut lo = 0usize;
        let mut hi = self.count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let base = HEADER_LEN + mid * ENTRY_LEN;
            let coord = (
                read_u32(&self.bytes, base),
                read_u32(&self.bytes, base + 4),
                read_u32(&self.bytes, base + 8),
            );
            match coord.cmp(&target) {
                std::cmp::Ordering::Equal => return Some(mid),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    fn entry_at(&self, idx: usize) -> (u32, u32, u32, usize, usize) {
        let base = HEADER_LEN + idx * ENTRY_LEN;
        (
            read_u32(&self.bytes, base),
            read_u32(&self.bytes, base + 4),
            read_u32(&self.bytes, base + 8),
            read_u32(&self.bytes, base + 12) as usize,
            read_u32(&self.bytes, base + 16) as usize,
        )
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn numeric_children(dir: &Path, suffix: &str) -> io::Result<Vec<(u32, std::path::PathBuf)>> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(stem) = name.strip_suffix(suffix) else {
            continue;
        };
        if !stem.is_empty()
            && stem.bytes().all(|b| b.is_ascii_digit())
            && let Ok(n) = stem.parse()
        {
            out.push((n, path));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_parse_and_binary_search_round_trip() {
        let raw = TileBundle::encode(&[
            (6, 10, 20, b"tile-b".to_vec()),
            (4, 2, 3, b"tile-a".to_vec()),
            (6, 10, 21, b"tile-c".to_vec()),
        ])
        .unwrap();
        let bundle = TileBundle::parse(Bytes::from(raw)).unwrap();
        assert_eq!(bundle.tile_count(), 3);
        assert_eq!(bundle.get(4, 2, 3).as_deref(), Some(b"tile-a".as_slice()));
        assert_eq!(bundle.get(6, 10, 20).as_deref(), Some(b"tile-b".as_slice()));
        assert_eq!(bundle.get(6, 10, 21).as_deref(), Some(b"tile-c".as_slice()));
        assert_eq!(bundle.get(5, 0, 0), None);
        assert!(bundle.contains(4, 2, 3));
        assert!(!bundle.contains(4, 2, 4));
    }

    #[test]
    fn empty_bundle_is_valid_and_answers_none() {
        let raw = TileBundle::encode(&[]).unwrap();
        let bundle = TileBundle::parse(Bytes::from(raw)).unwrap();
        assert_eq!(bundle.tile_count(), 0);
        assert_eq!(bundle.get(0, 0, 0), None);
    }

    #[test]
    fn from_dir_and_unpack_round_trip() {
        let src = tempfile::tempdir().unwrap();
        fs::create_dir_all(src.path().join("4/1")).unwrap();
        fs::write(src.path().join("4/1/2.png"), b"\x89PNG-a").unwrap();
        fs::write(src.path().join("4/1/3.png"), b"\x89PNG-b").unwrap();
        fs::write(src.path().join(".render-complete-v2.json"), b"{}").unwrap();

        let raw = TileBundle::from_dir(src.path()).unwrap();
        let bundle = TileBundle::parse(Bytes::from(raw)).unwrap();
        assert_eq!(bundle.tile_count(), 2);

        let dst = tempfile::tempdir().unwrap();
        bundle.unpack_to_dir(dst.path()).unwrap();
        assert_eq!(
            fs::read(dst.path().join("4/1/2.png")).unwrap(),
            b"\x89PNG-a"
        );
        assert_eq!(
            fs::read(dst.path().join("4/1/3.png")).unwrap(),
            b"\x89PNG-b"
        );
    }

    #[test]
    fn corrupt_bundles_are_rejected() {
        assert!(TileBundle::parse(Bytes::from_static(b"BAD")).is_err());
        assert!(TileBundle::parse(Bytes::from_static(b"RTB1\x02\0\0\0\0\0\0\0")).is_err());
        // Truncated index
        assert!(TileBundle::parse(Bytes::from_static(b"RTB1\x01\0\0\0\x01\0\0\0")).is_err());
    }
}
