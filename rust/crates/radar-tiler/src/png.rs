//! Indexed (palette-mode) PNG encoding and palette rewrite (spec-tiler section 7).
//!
//! A tile is encoded once as a PNG whose IDAT stream holds the class indices. Every other
//! palette of the same frame reuses that IDAT and gets a new PLTE/tRNS pair, which is a few
//! hundred bytes of chunk rewriting instead of a re-encode (spec 7.3).
//!
//! Bytes do not match Pillow or zlib-ng output (spec H6). Decoded pixels do. The zlib level is
//! fixed by configuration (`TILE_PNG_COMPRESS_LEVEL`), and the filter per row is chosen by
//! the minimum-sum-of-absolute-values heuristic.

use flate2::{Compress, Compression, FlushCompress, Status};

use crate::error::{Result, TilerError};

/// The eight-byte PNG signature.
pub const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
const IEND_CHUNK: [u8; 12] = [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
const COLOR_TYPE_INDEXED: u8 = 3;

/// Bit depth for an indexed image with `entries` palette entries (spec 7.1):
/// `<= 2` gives 1, `<= 4` gives 2, `<= 16` gives 4, otherwise 8.
pub fn bit_depth_for_entries(entries: usize) -> u8 {
    match entries {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
    }
}

/// Build one PNG chunk: `length (u32 BE) | type | payload | crc32(type || payload) (u32 BE)`.
pub fn make_chunk(ctype: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 12);
    append_chunk(&mut out, ctype, payload);
    out
}

fn append_chunk(out: &mut Vec<u8>, ctype: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(ctype);
    out.extend_from_slice(payload);
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(ctype);
    hasher.update(payload);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
}

/// Walk the chunks of a PNG, validating structure but not CRCs (`iter_chunks`).
pub fn iter_chunks(png: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    if !png.starts_with(&PNG_SIGNATURE) {
        return Err(TilerError::Invalid("not a PNG: bad signature".into()));
    }
    let mut out = Vec::new();
    let mut pos = PNG_SIGNATURE.len();
    while pos < png.len() {
        if pos + 8 > png.len() {
            return Err(TilerError::Invalid("truncated PNG chunk header".into()));
        }
        let length = u32::from_be_bytes(png[pos..pos + 4].try_into().expect("4 bytes")) as usize;
        let ctype: [u8; 4] = png[pos + 4..pos + 8].try_into().expect("4 bytes");
        let payload_start = pos + 8;
        let payload_end = payload_start.saturating_add(length);
        if payload_end.checked_add(4).is_none_or(|end| end > png.len()) {
            return Err(TilerError::Invalid(format!(
                "truncated PNG chunk {:?}",
                String::from_utf8_lossy(&ctype)
            )));
        }
        out.push((ctype, &png[payload_start..payload_end]));
        pos = payload_end + 4;
    }
    Ok(out)
}

/// IHDR chunk plus signature, the 33 bytes that precede every palette variant of a tile.
pub fn header_bytes(width: u32, height: u32, depth: u8) -> [u8; 33] {
    let mut out = [0u8; 33];
    out[..8].copy_from_slice(&PNG_SIGNATURE);
    let mut payload = [0u8; 13];
    payload[0..4].copy_from_slice(&width.to_be_bytes());
    payload[4..8].copy_from_slice(&height.to_be_bytes());
    payload[8] = depth;
    payload[9] = COLOR_TYPE_INDEXED;
    // compression, filter and interlace stay zero
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(b"IHDR");
    hasher.update(&payload);
    let mut chunk = Vec::with_capacity(25);
    chunk.extend_from_slice(&13u32.to_be_bytes());
    chunk.extend_from_slice(b"IHDR");
    chunk.extend_from_slice(&payload);
    chunk.extend_from_slice(&hasher.finalize().to_be_bytes());
    out[8..33].copy_from_slice(&chunk);
    out
}

/// Append the PLTE chunk, then the tRNS chunk, for one palette to `out`.
pub fn append_palette_chunks(out: &mut Vec<u8>, plte: &[u8], trns: &[u8]) {
    append_chunk(out, b"PLTE", plte);
    append_chunk(out, b"tRNS", trns);
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let (pa, pb, pc) = ((p - ia).abs(), (p - ib).abs(), (p - ic).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

#[inline]
fn filter_byte(kind: u8, x: u8, a: u8, b: u8, c: u8) -> u8 {
    match kind {
        0 => x,
        1 => x.wrapping_sub(a),
        2 => x.wrapping_sub(b),
        3 => x.wrapping_sub(((u16::from(a) + u16::from(b)) / 2) as u8),
        _ => x.wrapping_sub(paeth(a, b, c)),
    }
}

/// Sum of absolute signed values for one filter choice (the selection heuristic).
fn filter_score(kind: u8, cur: &[u8], prev: &[u8]) -> u64 {
    let mut total = 0u64;
    let mut a = 0u8;
    let mut c = 0u8;
    for (i, &x) in cur.iter().enumerate() {
        let b = prev[i];
        total += u64::from((filter_byte(kind, x, a, b, c) as i8).unsigned_abs());
        a = x;
        c = b;
    }
    total
}

/// Reusable encoder state: the zlib compressor and scratch buffers. Keep one per worker thread.
pub struct IndexedTileEncoder {
    level: u32,
    compressor: Compress,
    cur: Vec<u8>,
    prev: Vec<u8>,
    raw: Vec<u8>,
}

impl IndexedTileEncoder {
    /// Create an encoder with zlib `level` (0 through 9).
    pub fn new(level: u32) -> Self {
        Self {
            level,
            compressor: Compress::new(Compression::new(level), true),
            cur: Vec::new(),
            prev: Vec::new(),
            raw: Vec::new(),
        }
    }

    /// Encode the IDAT and IEND chunks for `tile` (row-major class indices, `width * height`)
    /// into `out`, replacing its contents. The header and palette chunks are separate.
    pub fn encode_tail(
        &mut self,
        tile: &[u8],
        width: usize,
        height: usize,
        depth: u8,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        debug_assert_eq!(tile.len(), width * height);
        self.build_scanlines(tile, width, height, depth);
        out.clear();
        out.extend_from_slice(&[0u8; 8]); // length and type, patched below
        self.compressor.reset();
        deflate_into(&mut self.compressor, &self.raw, out)?;
        let data_len = (out.len() - 8) as u32;
        out[0..4].copy_from_slice(&data_len.to_be_bytes());
        out[4..8].copy_from_slice(b"IDAT");
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&out[4..]);
        out.extend_from_slice(&hasher.finalize().to_be_bytes());
        out.extend_from_slice(&IEND_CHUNK);
        Ok(())
    }

    /// The zlib level this encoder was built with.
    pub fn level(&self) -> u32 {
        self.level
    }

    fn build_scanlines(&mut self, tile: &[u8], width: usize, height: usize, depth: u8) {
        let depth = usize::from(depth);
        let per_byte = 8 / depth;
        let row_bytes = (width * depth).div_ceil(8);
        self.cur.clear();
        self.cur.resize(row_bytes, 0);
        self.prev.clear();
        self.prev.resize(row_bytes, 0);
        self.raw.clear();
        self.raw.reserve(height * (row_bytes + 1));
        for y in 0..height {
            let src = &tile[y * width..(y + 1) * width];
            self.cur.fill(0);
            for (j, &v) in src.iter().enumerate() {
                let shift = 8 - depth * (j % per_byte + 1);
                self.cur[j / per_byte] |= v << shift;
            }
            let mut best = 0u8;
            let mut best_score = u64::MAX;
            for kind in 0..5u8 {
                let score = filter_score(kind, &self.cur, &self.prev);
                if score < best_score {
                    best_score = score;
                    best = kind;
                }
            }
            self.raw.push(best);
            let (cur, prev) = (&self.cur, &self.prev);
            let mut a = 0u8;
            let mut c = 0u8;
            for (i, &x) in cur.iter().enumerate() {
                let b = prev[i];
                self.raw.push(filter_byte(best, x, a, b, c));
                a = x;
                c = b;
            }
            std::mem::swap(&mut self.cur, &mut self.prev);
        }
    }
}

fn deflate_into(compressor: &mut Compress, mut input: &[u8], out: &mut Vec<u8>) -> Result<()> {
    loop {
        if out.capacity() - out.len() < 4096 {
            let grow = out.capacity().max(16 * 1024);
            out.reserve(grow);
        }
        let before = compressor.total_in();
        let status = compressor
            .compress_vec(input, out, FlushCompress::Finish)
            .map_err(|e| TilerError::Invalid(format!("zlib compression failed: {e}")))?;
        input = &input[(compressor.total_in() - before) as usize..];
        if status == Status::StreamEnd {
            return Ok(());
        }
    }
}

/// Encode a complete indexed PNG for `tile` with the given palette (spec `encode_indexed_png`).
///
/// `plte` is 3 bytes per entry (1 to 256 entries) and `trns` is one alpha byte per entry.
pub fn encode_indexed(
    tile: &[u8],
    width: usize,
    height: usize,
    plte: &[u8],
    trns: &[u8],
    level: u32,
) -> Result<Vec<u8>> {
    validate_palette(plte, Some(trns))?;
    if tile.len() != width * height {
        return Err(TilerError::Invalid(
            "tile length must equal width * height".into(),
        ));
    }
    let entries = plte.len() / 3;
    let depth = bit_depth_for_entries(entries);
    let mut enc = IndexedTileEncoder::new(level);
    let mut tail = Vec::new();
    enc.encode_tail(tile, width, height, depth, &mut tail)?;
    let mut out = Vec::with_capacity(tail.len() + plte.len() + trns.len() + 128);
    out.extend_from_slice(&header_bytes(width as u32, height as u32, depth));
    append_palette_chunks(&mut out, plte, trns);
    out.extend_from_slice(&tail);
    Ok(out)
}

fn validate_palette(rgb: &[u8], alpha: Option<&[u8]>) -> Result<()> {
    if !rgb.len().is_multiple_of(3) || !(1..=256).contains(&(rgb.len() / 3)) {
        return Err(TilerError::Invalid(
            "rgb must be 3 bytes per entry, 1..256 entries".into(),
        ));
    }
    if let Some(alpha) = alpha
        && alpha.len() > rgb.len() / 3
    {
        return Err(TilerError::Invalid(
            "alpha has more entries than the palette".into(),
        ));
    }
    Ok(())
}

/// Return `png` with its PLTE (and tRNS) replaced; IDAT and every other chunk are kept with
/// fresh CRCs (spec 7.3). `alpha = None` drops the tRNS chunk.
pub fn replace_palette(png: &[u8], rgb: &[u8], alpha: Option<&[u8]>) -> Result<Vec<u8>> {
    validate_palette(rgb, alpha)?;
    let mut out = Vec::with_capacity(png.len() + rgb.len() + 64);
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut saw_plte = false;
    for (ctype, payload) in iter_chunks(png)? {
        match &ctype {
            b"IHDR" => {
                let color_type = payload.get(9).copied().unwrap_or(0);
                if color_type != COLOR_TYPE_INDEXED {
                    return Err(TilerError::Invalid(format!(
                        "PNG colour type {color_type} is not indexed (3)"
                    )));
                }
                append_chunk(&mut out, &ctype, payload);
            }
            b"PLTE" => {
                saw_plte = true;
                append_chunk(&mut out, b"PLTE", rgb);
                if let Some(alpha) = alpha {
                    append_chunk(&mut out, b"tRNS", alpha);
                }
            }
            b"tRNS" => {
                if !saw_plte {
                    return Err(TilerError::Invalid("tRNS before PLTE".into()));
                }
                // The original tRNS is dropped; the new one was emitted right after PLTE.
            }
            b"IDAT" if !saw_plte => {
                return Err(TilerError::Invalid("indexed PNG has no PLTE chunk".into()));
            }
            _ => append_chunk(&mut out, &ctype, payload),
        }
    }
    if !saw_plte {
        return Err(TilerError::Invalid("indexed PNG has no PLTE chunk".into()));
    }
    Ok(out)
}

/// Decode an indexed PNG's raw class indices (one byte per pixel) and its palette, for tests
/// and the parity tools. Only colour type 3 is accepted.
pub fn decode_indexed(png: &[u8]) -> Result<DecodedIndexed> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder
        .read_info()
        .map_err(|e| TilerError::Corrupt(format!("png decode: {e}")))?;
    let info = reader.info().clone();
    if info.color_type != png::ColorType::Indexed {
        return Err(TilerError::Corrupt("not an indexed PNG".into()));
    }
    let buffer_len = reader
        .output_buffer_size()
        .ok_or_else(|| TilerError::Corrupt("png output buffer too large".into()))?;
    let mut raw = vec![0u8; buffer_len];
    let frame = reader
        .next_frame(&mut raw)
        .map_err(|e| TilerError::Corrupt(format!("png decode: {e}")))?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let depth = info.bit_depth as usize;
    let per_byte = 8 / depth;
    let row_bytes = (w * depth).div_ceil(8);
    let mask = ((1u16 << depth) - 1) as u8;
    let mut indices = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &raw[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..w {
            let shift = 8 - depth * (x % per_byte + 1);
            indices.push((row[x / per_byte] >> shift) & mask);
        }
    }
    let plte = info.palette.as_deref().unwrap_or(&[]).to_vec();
    let trns = info.trns.as_deref().unwrap_or(&[]).to_vec();
    Ok(DecodedIndexed {
        width: w,
        height: h,
        indices,
        plte,
        trns,
    })
}

/// Pixels and palette of a decoded indexed PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedIndexed {
    /// Image width.
    pub width: usize,
    /// Image height.
    pub height: usize,
    /// One class index per pixel, row-major.
    pub indices: Vec<u8>,
    /// PLTE payload (3 bytes per entry).
    pub plte: Vec<u8>,
    /// tRNS payload (one alpha byte per entry), empty when absent.
    pub trns: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tile(w: usize, h: usize, classes: usize) -> Vec<u8> {
        (0..w * h)
            .map(|i| ((i * 7 + i / w) % classes) as u8)
            .collect()
    }

    fn palette(n: usize) -> (Vec<u8>, Vec<u8>) {
        let plte: Vec<u8> = (0..n)
            .flat_map(|i| [i as u8, (255 - i) as u8, (i * 3) as u8])
            .collect();
        let trns: Vec<u8> = (0..n).map(|i| if i == 0 { 0 } else { 255 }).collect();
        (plte, trns)
    }

    #[test]
    fn bit_depth_boundaries() {
        assert_eq!(bit_depth_for_entries(1), 1);
        assert_eq!(bit_depth_for_entries(2), 1);
        assert_eq!(bit_depth_for_entries(3), 2);
        assert_eq!(bit_depth_for_entries(4), 2);
        assert_eq!(bit_depth_for_entries(5), 4);
        assert_eq!(bit_depth_for_entries(16), 4);
        assert_eq!(bit_depth_for_entries(17), 8);
        assert_eq!(bit_depth_for_entries(256), 8);
    }

    #[test]
    fn encode_then_decode_roundtrips_pixels_at_every_depth() {
        for n in [1usize, 2, 3, 5, 12, 16, 17, 200, 256] {
            let tile = sample_tile(37, 19, n);
            let (plte, trns) = palette(n);
            let png = encode_indexed(&tile, 37, 19, &plte, &trns, 6).unwrap();
            let dec = decode_indexed(&png).unwrap();
            assert_eq!(dec.indices, tile, "entries {n}");
            assert_eq!(dec.plte, plte);
            assert_eq!(dec.trns, trns);
        }
    }

    #[test]
    fn chunk_order_is_ihdr_plte_trns_idat_iend() {
        let tile = sample_tile(256, 256, 4);
        let (plte, trns) = palette(4);
        let png = encode_indexed(&tile, 256, 256, &plte, &trns, 6).unwrap();
        let types: Vec<String> = iter_chunks(&png)
            .unwrap()
            .iter()
            .map(|(t, _)| String::from_utf8_lossy(t).into())
            .collect();
        assert_eq!(types, ["IHDR", "PLTE", "tRNS", "IDAT", "IEND"]);
    }

    /// Spec 7.3 guarantee: decoded pixels after rewrite equal a direct encode with the new palette,
    /// and IDAT and IHDR payloads are byte-identical to the source.
    #[test]
    fn replace_palette_matches_direct_encode() {
        for n in [2usize, 12, 16, 17, 200] {
            let tile = sample_tile(64, 64, n);
            let (plte_a, trns_a) = palette(n);
            let base = encode_indexed(&tile, 64, 64, &plte_a, &trns_a, 6).unwrap();
            let (plte_b, trns_b) = {
                let (p, _) = palette(n);
                (
                    p.iter().map(|v| v ^ 0x55).collect::<Vec<u8>>(),
                    (0..n).map(|i| (i % 2 * 255) as u8).collect::<Vec<u8>>(),
                )
            };
            let rewritten = replace_palette(&base, &plte_b, Some(&trns_b)).unwrap();
            let direct = encode_indexed(&tile, 64, 64, &plte_b, &trns_b, 6).unwrap();
            assert_eq!(
                decode_indexed(&rewritten).unwrap(),
                decode_indexed(&direct).unwrap(),
                "entries {n}"
            );
            let idat_of = |b: &[u8]| {
                iter_chunks(b)
                    .unwrap()
                    .into_iter()
                    .find(|(t, _)| t == b"IDAT")
                    .unwrap()
                    .1
                    .to_vec()
            };
            assert_eq!(idat_of(&rewritten), idat_of(&base));
            // Our fast path produces identical bytes to the generic rewrite.
            assert_eq!(rewritten, direct, "byte-identical to the one-shot encode");
        }
    }

    #[test]
    fn every_chunk_crc_is_fresh() {
        let tile = sample_tile(16, 16, 3);
        let (plte, trns) = palette(3);
        let png = replace_palette(
            &encode_indexed(&tile, 16, 16, &plte, &trns, 1).unwrap(),
            &plte,
            None,
        )
        .unwrap();
        let mut pos = 8;
        while pos < png.len() {
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let body = &png[pos + 4..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            assert_eq!(crc, crc32fast::hash(body));
            pos += 12 + len;
        }
    }

    #[test]
    fn replace_palette_inserts_and_drops_trns() {
        let tile = sample_tile(8, 8, 2);
        let (plte, trns) = palette(2);
        let base = encode_indexed(&tile, 8, 8, &plte, &trns, 6).unwrap();
        let with = replace_palette(&base, &plte, Some(&[0u8])).unwrap();
        let chunks = iter_chunks(&with).unwrap();
        assert!(chunks.iter().any(|(t, p)| t == b"tRNS" && *p == [0u8]));
        let without = replace_palette(&base, &plte, None).unwrap();
        assert!(
            !iter_chunks(&without)
                .unwrap()
                .iter()
                .any(|(t, _)| t == b"tRNS")
        );
        let dec = decode_indexed(&with).unwrap();
        assert_eq!(dec.trns, vec![0u8]);
    }

    #[test]
    fn replace_palette_rejects_non_indexed_and_bad_lengths() {
        // RGBA PNG: colour type 6 in IHDR.
        let mut rgba = Vec::from(PNG_SIGNATURE);
        let mut ihdr = vec![0u8; 13];
        ihdr[8] = 8;
        ihdr[9] = 6;
        rgba.extend(make_chunk(b"IHDR", &ihdr));
        rgba.extend(make_chunk(b"IEND", &[]));
        let err = replace_palette(&rgba, &[0, 0, 0], None).unwrap_err();
        assert!(err.to_string().contains("not indexed"));
        // rgb of 4 bytes, alpha longer than the palette, and a bad signature.
        assert!(replace_palette(&rgba, &[0, 0, 0, 0], None).is_err());
        assert!(replace_palette(&rgba, &[0, 0, 0], Some(&[1, 2])).is_err());
        assert!(replace_palette(b"nope", &[0, 0, 0], None).is_err());
    }

    #[test]
    fn iter_chunks_detects_truncation() {
        let mut ok = Vec::from(PNG_SIGNATURE);
        ok.extend_from_slice(&IEND_CHUNK);
        let chunks = iter_chunks(&ok).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(&chunks[0].0, b"IEND");
        let truncated = &ok[..ok.len() - 3];
        let err = iter_chunks(truncated).unwrap_err();
        assert!(err.to_string().contains("truncated"));
    }
}
