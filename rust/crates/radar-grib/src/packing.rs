//! Exact GRIB2 data unpacking in f64, for data representation templates 5.0, 5.2, 5.3 and 5.41.
//!
//! Why this module exists: the `grib` crate scales integers in f32, `(R + X*2^E) * 10^-D`, while
//! eccodes (which pygrib uses) scales in f64 and casts the result to f32 once. The two differ by one
//! ulp for a noticeable fraction of packed values, and a one-ulp change can move a value across a
//! palette class boundary. This module recovers the integer values X exactly and applies the same
//! scale formula in f64, so the f32 result is the same as `np.float32(pygrib.values)`.
//!
//! The complex-packing layout follows the `grib` crate's decoder (group references, widths and
//! lengths, each padded to whole octets, then one contiguous bit stream of group values). The
//! spatial-differencing reconstruction also follows that decoder. Descriptors use sign-magnitude
//! (WMO FM 92), which the crate does not do for 1- and 2-byte fields. The eccodes golden fixtures
//! confirm the choices. Templates 5.40, 5.42 and 5.200 are not handled here; `decode.rs` falls back
//! to the `grib` crate for them.

use crate::error::{GribError, Result};

/// Reads `n` bits MSB-first from a byte slice, starting at bit 0.
pub(crate) struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Reads `n` bits (n <= 57) as an unsigned integer.
    pub(crate) fn read(&mut self, n: u32) -> Result<u64> {
        if n == 0 {
            return Ok(0);
        }
        let end = self.pos + n as usize;
        if end > self.data.len() * 8 {
            return Err(GribError::Decode("packed data ends early".into()));
        }
        // Fast path for whole bytes at a byte boundary.
        if n.is_multiple_of(8) && self.pos.is_multiple_of(8) {
            let start = self.pos / 8;
            let bytes = &self.data[start..start + (n as usize / 8)];
            let v = bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
            self.pos = end;
            return Ok(v);
        }
        let mut v = 0u64;
        for _ in 0..n {
            let byte = self.data[self.pos >> 3];
            let bit = (byte >> (7 - (self.pos & 7))) & 1;
            v = (v << 1) | u64::from(bit);
            self.pos += 1;
        }
        Ok(v)
    }
}

/// Reads a WMO sign-magnitude integer of `bytes.len()` bytes (1..=4).
pub(crate) fn sign_magnitude(bytes: &[u8]) -> i64 {
    let mut v: u64 = 0;
    for b in bytes {
        v = (v << 8) | u64::from(*b);
    }
    let bits = 8 * bytes.len() as u32;
    let sign = 1u64 << (bits - 1);
    if v & sign != 0 {
        -((v & (sign - 1)) as i64)
    } else {
        v as i64
    }
}

/// Scaling parameters of a data representation section (octets 12-21).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Scale {
    /// Reference value R (IEEE 754 binary32, octets 12-15).
    pub r: f64,
    /// Binary scale factor E (octets 16-17, sign-magnitude).
    pub e: i32,
    /// Decimal scale factor D (octets 18-19, sign-magnitude).
    pub d: i32,
    /// Bits per packed value (octet 20).
    pub nbits: u32,
}

impl Scale {
    /// Reads the common scaling fields from a complete section 5 (header included).
    pub(crate) fn from_section5(s5: &[u8]) -> Result<Self> {
        if s5.len() < 21 {
            return Err(GribError::Parse("section 5 shorter than 21 octets".into()));
        }
        let r = f32::from_be_bytes([s5[11], s5[12], s5[13], s5[14]]);
        Ok(Self {
            r: f64::from(r),
            e: sign_magnitude(&s5[15..17]) as i32,
            d: sign_magnitude(&s5[17..19]) as i32,
            nbits: u32::from(s5[19]),
        })
    }

    /// Scales one packed integer: `(R + X * 2^E) * 10^-D`, in f64, using the factors from [`Scale::factors`].
    #[inline]
    pub(crate) fn apply(&self, x: i64, binary: f64, decimal: f64) -> f64 {
        (self.r + x as f64 * binary) * decimal
    }

    /// `2^E` and `10^-D` as eccodes computes them (libm `pow`).
    pub(crate) fn factors(&self) -> (f64, f64) {
        (2f64.powf(f64::from(self.e)), 10f64.powf(-f64::from(self.d)))
    }
}

/// Octet-level fields of a complex-packing section 5 (templates 5.2 and 5.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Complex {
    /// Missing value management (octet 23, Code table 5.5): 0 none, 1 primary, 2 primary+secondary.
    pub mvm: u8,
    /// Number of groups NG (octets 32-35).
    pub ng: usize,
    /// Reference for group widths (octet 36).
    pub width_ref: u32,
    /// Bits per group width (octet 37).
    pub width_bits: u32,
    /// Reference for group lengths (octets 38-41).
    pub len_ref: u32,
    /// Increment for group lengths (octet 42).
    pub len_inc: u32,
    /// True length of the last group (octets 43-46).
    pub len_last: u32,
    /// Bits per scaled group length (octet 47).
    pub len_bits: u32,
    /// Order of spatial differencing (octet 48, template 5.3 only; 0 for 5.2).
    pub spatial_order: u8,
    /// Octets per spatial-differencing descriptor (octet 49, template 5.3 only).
    pub spatial_octets: u8,
}

impl Complex {
    /// Parses the complex fields from a complete section 5 of template 5.2 or 5.3.
    pub(crate) fn from_section5(s5: &[u8], spatial: bool) -> Result<Self> {
        let need = if spatial { 49 } else { 47 };
        if s5.len() < need {
            return Err(GribError::Parse(format!(
                "section 5 shorter than {need} octets"
            )));
        }
        if s5[21] != 1 {
            return Err(GribError::Unsupported(format!(
                "group splitting method {}",
                s5[21]
            )));
        }
        let mvm = s5[22];
        if mvm > 2 {
            return Err(GribError::Unsupported(format!(
                "missing value management {mvm}"
            )));
        }
        let be32 = |i: usize| u32::from_be_bytes([s5[i], s5[i + 1], s5[i + 2], s5[i + 3]]);
        Ok(Self {
            mvm,
            ng: be32(31) as usize,
            width_ref: u32::from(s5[35]),
            width_bits: u32::from(s5[36]),
            len_ref: be32(37),
            len_inc: u32::from(s5[41]),
            len_last: be32(42),
            len_bits: u32::from(s5[46]),
            spatial_order: if spatial { s5[47] } else { 0 },
            spatial_octets: if spatial { s5[48] } else { 0 },
        })
    }
}

/// Unpacks a simple-packing (5.0) or PNG (5.41) field: `n` integers from `payload`.
pub(crate) fn unpack_simple(payload: &[u8], nbits: u32, n: usize) -> Result<Vec<i64>> {
    if nbits == 0 {
        return Ok(vec![0; n]);
    }
    let mut bits = BitReader::new(payload);
    (0..n).map(|_| bits.read(nbits).map(|v| v as i64)).collect()
}

/// Unpacks a complex-packing field (5.2 or 5.3) into integers. `None` marks a missing value.
///
/// `payload` is section 7 without its 5-byte header. Output length is `n_encoded`.
pub(crate) fn unpack_complex(
    c: &Complex,
    nbits: u32,
    payload: &[u8],
    n_encoded: usize,
) -> Result<Vec<Option<i64>>> {
    let ng = c.ng;
    if ng == 0 {
        return Err(GribError::Decode("complex packing with zero groups".into()));
    }
    let mut off = 0usize;

    // Spatial-differencing descriptors: `order` first values, then the overall minimum.
    let mut first_values: Vec<i64> = Vec::new();
    let mut z_min = 0i64;
    if c.spatial_order != 0 {
        let order = usize::from(c.spatial_order);
        let octets = usize::from(c.spatial_octets);
        if !(1..=2).contains(&order) || !(1..=4).contains(&octets) {
            return Err(GribError::Unsupported(format!(
                "spatial differencing order {order} with {octets} descriptor octets"
            )));
        }
        let plen = (order + 1) * octets;
        let p = payload
            .get(..plen)
            .ok_or_else(|| GribError::Decode("section 7 shorter than its descriptors".into()))?;
        first_values = (0..order)
            .map(|k| sign_magnitude(&p[k * octets..(k + 1) * octets]))
            .collect();
        z_min = sign_magnitude(&p[order * octets..plen]);
        off = plen;
    }

    // Group references (NG values of nbits), padded to octets.
    let ref_len = (ng * nbits as usize).div_ceil(8);
    let refs = read_fixed(payload, off, ref_len, ng, nbits)?;
    off += ref_len;

    // Group widths (NG values of width_bits, plus width_ref), padded to octets.
    let width_len = (ng * c.width_bits as usize).div_ceil(8);
    let widths: Vec<u64> = read_fixed(payload, off, width_len, ng, c.width_bits)?
        .into_iter()
        .map(|w| w + u64::from(c.width_ref))
        .collect();
    off += width_len;

    // Group lengths: NG-1 scaled values (len_bits each, times len_inc plus len_ref), then len_last.
    let len_len = (ng * c.len_bits as usize).div_ceil(8);
    let scaled = read_fixed(payload, off, len_len, ng.saturating_sub(1), c.len_bits)?;
    let mut lengths: Vec<usize> = scaled
        .into_iter()
        .map(|v| (v * u64::from(c.len_inc) + u64::from(c.len_ref)) as usize)
        .collect();
    lengths.push(c.len_last as usize);
    off += len_len;

    let total: usize = lengths.iter().sum();
    if total != n_encoded {
        return Err(GribError::Decode(format!(
            "complex packing groups cover {total} values, expected {n_encoded}"
        )));
    }

    // Group values: one contiguous bit stream; constant groups (width 0) consume no bits.
    let data = payload
        .get(off..)
        .ok_or_else(|| GribError::Decode("section 7 ends before group values".into()))?;
    let mut bits = BitReader::new(data);
    let missing1_ref = (1u64 << nbits) - 1;
    let missing2_ref = missing1_ref.saturating_sub(1);
    let mut out: Vec<Option<i64>> = Vec::with_capacity(n_encoded);
    for g in 0..ng {
        let r = refs[g];
        let w = widths[g] as u32;
        let len = lengths[g];
        if w == 0 {
            let missing = (c.mvm > 0 && r == missing1_ref) || (c.mvm == 2 && r == missing2_ref);
            let v = if missing {
                None
            } else {
                Some(r as i64 + z_min)
            };
            out.extend(std::iter::repeat_n(v, len));
            continue;
        }
        let m1 = (1u64 << w) - 1;
        let m2 = m1 - 1;
        for _ in 0..len {
            let v = bits.read(w)?;
            if (c.mvm > 0 && v == m1) || (c.mvm == 2 && v == m2) {
                out.push(None);
            } else {
                out.push(Some((v + r) as i64 + z_min));
            }
        }
    }

    if c.spatial_order != 0 {
        out = reconstruct(out, &first_values, c.spatial_order);
    }
    Ok(out)
}

/// Undoes spatial differencing over the non-missing values, as the `grib` crate does.
fn reconstruct(vals: Vec<Option<i64>>, first: &[i64], order: u8) -> Vec<Option<i64>> {
    let mut out = Vec::with_capacity(vals.len());
    let mut count = 0usize;
    let (mut prev1, mut prev2) = (0i64, 0i64);
    for v in vals {
        let Some(d) = v else {
            out.push(None);
            continue;
        };
        let value = if order == 1 {
            if count == 0 {
                prev1 = first[0];
                prev1
            } else {
                prev1 += d;
                prev1
            }
        } else {
            match count {
                0 => {
                    prev2 = first[0];
                    prev2
                }
                1 => {
                    prev1 = first[1];
                    prev1
                }
                _ => {
                    let v = d + 2 * prev1 - prev2;
                    prev2 = prev1;
                    prev1 = v;
                    v
                }
            }
        };
        count += 1;
        out.push(Some(value));
    }
    out
}

/// Reads `count` fixed-width unsigned values from `payload[off..off+len]`.
fn read_fixed(
    payload: &[u8],
    off: usize,
    len: usize,
    count: usize,
    width: u32,
) -> Result<Vec<u64>> {
    let slice = payload
        .get(off..off + len)
        .ok_or_else(|| GribError::Decode("section 7 ends inside group descriptors".into()))?;
    let mut bits = BitReader::new(slice);
    (0..count).map(|_| bits.read(width)).collect()
}

/// Expands the packed values through the section 6 bitmap (or none) into `n` f32 values.
///
/// `indicator` is section 6 octet 6: 255 means no bitmap, 0 means a bitmap follows.
/// Missing (in-line or bitmap) values become NaN.
pub(crate) fn expand_bitmap(packed: Vec<f64>, s6: Option<&[u8]>, n: usize) -> Result<Vec<f32>> {
    let indicator = s6.map(|s| s[5]).unwrap_or(255);
    match indicator {
        255 => {
            if packed.len() != n {
                return Err(GribError::Decode(format!(
                    "{} values for {n} points",
                    packed.len()
                )));
            }
            Ok(packed.into_iter().map(|v| v as f32).collect())
        }
        0 => {
            let bm = &s6.unwrap()[6..];
            let mut it = packed.into_iter();
            let mut out = Vec::with_capacity(n);
            for i in 0..n {
                let bit = bm.get(i / 8).map(|b| (b >> (7 - (i % 8))) & 1).unwrap_or(0);
                if bit == 1 {
                    let v = it.next().ok_or_else(|| {
                        GribError::Decode("bitmap has more set bits than values".into())
                    })?;
                    out.push(v as f32);
                } else {
                    out.push(f32::NAN);
                }
            }
            if it.next().is_some() {
                return Err(GribError::Decode(
                    "bitmap has fewer set bits than values".into(),
                ));
            }
            Ok(out)
        }
        other => Err(GribError::Unsupported(format!(
            "bitmap indicator {other} (predefined bitmap)"
        ))),
    }
}

/// Reads the pixel integers of a PNG-packed field (template 5.41).
///
/// Each row starts on a byte boundary; `nbits` must equal the PNG bit depth.
pub(crate) fn unpack_png(payload: &[u8], nbits: u32, n: usize) -> Result<Vec<i64>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(payload));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder
        .read_info()
        .map_err(|e| GribError::Decode(format!("PNG header: {e}")))?;
    let (width, height, depth, color) = {
        let info = reader.info();
        (
            info.width as usize,
            info.height as usize,
            info.bit_depth as u8,
            info.color_type,
        )
    };
    if color != png::ColorType::Grayscale {
        return Err(GribError::Unsupported(format!("PNG color type {color:?}")));
    }
    if u32::from(depth) != nbits {
        return Err(GribError::Decode(format!(
            "PNG depth {depth} differs from nbits {nbits}"
        )));
    }
    if width * height != n {
        return Err(GribError::Decode(format!(
            "PNG holds {} pixels, grid has {n}",
            width * height
        )));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| GribError::Decode("PNG output size unknown".into()))?;
    let mut buf = vec![0u8; size];
    reader
        .next_frame(&mut buf)
        .map_err(|e| GribError::Decode(format!("PNG decode: {e}")))?;
    let row_bytes = (width * depth as usize).div_ceil(8);
    let mut out = Vec::with_capacity(n);
    for r in 0..height {
        let row = &buf[r * row_bytes..(r + 1) * row_bytes];
        let mut br = BitReader::new(row);
        for _ in 0..width {
            out.push(br.read(nbits)? as i64);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_magnitude_reads_wmo_negatives() {
        assert_eq!(sign_magnitude(&[0x80, 0x03]), -3);
        assert_eq!(sign_magnitude(&[0x00, 0x03]), 3);
        assert_eq!(sign_magnitude(&[0x81]), -1);
        assert_eq!(sign_magnitude(&[0x80, 0x00, 0x00, 0x05]), -5);
    }

    #[test]
    fn bit_reader_msb_first() {
        let mut br = BitReader::new(&[0b1010_1100, 0b1111_0000]);
        assert_eq!(br.read(3).unwrap(), 0b101);
        assert_eq!(br.read(5).unwrap(), 0b01100);
        assert_eq!(br.read(4).unwrap(), 0b1111);
        assert!(br.read(8).is_err());
    }

    #[test]
    fn bitmap_expansion_places_nan() {
        // indicator 0, bitmap 1010_0000 -> positions 0 and 2 present
        let s6 = [0u8, 0, 0, 0, 0, 0, 0b1010_0000];
        let out = expand_bitmap(vec![1.0, 2.0], Some(&s6), 4).unwrap();
        assert_eq!(out[0], 1.0);
        assert!(out[1].is_nan());
        assert_eq!(out[2], 2.0);
        assert!(out[3].is_nan());
    }

    #[test]
    fn spatial_first_order_reconstruction_skips_missing() {
        let vals = vec![Some(0), None, Some(2), Some(3)];
        let out = reconstruct(vals, &[10], 1);
        assert_eq!(out, vec![Some(10), None, Some(12), Some(15)]);
    }
}
