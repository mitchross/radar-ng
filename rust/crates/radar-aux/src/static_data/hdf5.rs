//! A minimal HDF5 reader: just enough of the format to find a named dataset in the root group of a
//! netCDF-4 file and list where its raw chunks sit. No C library.
//!
//! Supported: superblock v2/v3, version 2 object headers with compact links, layout message v3
//! (contiguous or chunked with a version 1 B-tree), little-endian floats. Anything else is an error,
//! so a changed upstream file fails loudly instead of decoding garbage.

use std::io;

/// Random access to the file bytes (a local file, or HTTP range requests).
pub trait ReadAt {
    /// Exactly `len` bytes at `offset`.
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>>;
}

impl ReadAt for std::fs::File {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        use std::io::{Read, Seek, SeekFrom};
        self.seek(SeekFrom::Start(offset))?;
        let mut buf = vec![0u8; len];
        self.read_exact(&mut buf)?;
        Ok(buf)
    }
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn u16_at(b: &[u8], i: usize) -> io::Result<u16> {
    b.get(i..i + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or_else(|| bad("short read"))
}

fn u32_at(b: &[u8], i: usize) -> io::Result<u32> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes(s.try_into().expect("4 bytes")))
        .ok_or_else(|| bad("short read"))
}

fn u64_at(b: &[u8], i: usize) -> io::Result<u64> {
    b.get(i..i + 8)
        .map(|s| u64::from_le_bytes(s.try_into().expect("8 bytes")))
        .ok_or_else(|| bad("short read"))
}

fn uint_at(b: &[u8], i: usize, size: usize) -> io::Result<u64> {
    let s = b.get(i..i + size).ok_or_else(|| bad("short read"))?;
    Ok(s.iter()
        .rev()
        .fold(0u64, |acc, &x| (acc << 8) | u64::from(x)))
}

/// Where a dataset's raw data lives.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// One block of `size` bytes at `address`.
    Contiguous {
        /// File offset.
        address: u64,
        /// Byte length.
        size: u64,
    },
    /// Chunks indexed by a version 1 B-tree.
    Chunked {
        /// Root node of the chunk B-tree.
        btree: u64,
        /// Chunk shape in elements (without the trailing element-size dimension).
        chunk: Vec<u64>,
    },
}

/// One dataset of the root group.
#[derive(Debug, Clone, PartialEq)]
pub struct Dataset {
    /// Current dimension sizes.
    pub dims: Vec<u64>,
    /// Bytes per element.
    pub element_size: u32,
    /// True for a little-endian IEEE float.
    pub float_le: bool,
    /// True when a filter pipeline (compression) is present.
    pub filtered: bool,
    /// Raw data location.
    pub layout: Layout,
}

/// One raw chunk of a chunked dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    /// Element offset of the chunk's first element in each dimension.
    pub offset: [u64; 3],
    /// File offset of the chunk bytes.
    pub address: u64,
    /// Stored byte size.
    pub size: u32,
    /// Filters skipped for this chunk (0 when unfiltered).
    pub filter_mask: u32,
}

/// Header messages of the object header at `addr` as `(type, body)`, continuation blocks followed.
fn header_messages(r: &mut dyn ReadAt, addr: u64) -> io::Result<Vec<(u8, Vec<u8>)>> {
    let head = r.read_at(addr, 16)?;
    if &head[..4] != b"OHDR" || head[4] != 2 {
        return Err(bad(format!("no version 2 object header at {addr}")));
    }
    let flags = head[5];
    let mut i = 6;
    if flags & 0x20 != 0 {
        i += 16;
    }
    if flags & 0x10 != 0 {
        i += 4;
    }
    let size_len = 1usize << (flags & 3);
    let head = r.read_at(addr, i + size_len)?;
    let size0 = uint_at(&head, i, size_len)? as usize;
    let start = i + size_len;
    let first = r.read_at(addr, start + size0)?;
    let track_order = flags & 0x04 != 0;
    let mut blocks = vec![(first, start, start + size0)];
    let mut out = Vec::new();
    while let Some((buf, mut i, end)) = blocks.pop() {
        let msg_head = if track_order { 6 } else { 4 };
        while i + msg_head <= end {
            let t = buf[i];
            let sz = u16_at(&buf, i + 1)? as usize;
            i += msg_head;
            let body = buf
                .get(i..i + sz)
                .ok_or_else(|| bad("header message past end"))?
                .to_vec();
            i += sz;
            if t == 0x10 {
                let (a, l) = (u64_at(&body, 0)?, u64_at(&body, 8)? as usize);
                let cont = r.read_at(a, l)?;
                if cont.get(..4) != Some(b"OCHK") {
                    return Err(bad(format!("no continuation block at {a}")));
                }
                // The last 4 bytes are the checksum.
                blocks.push((cont, 4, l.saturating_sub(4)));
            }
            out.push((t, body));
        }
    }
    Ok(out)
}

/// Address of the root group's object header.
fn root_address(r: &mut dyn ReadAt) -> io::Result<u64> {
    let sb = r.read_at(0, 48)?;
    if &sb[..8] != b"\x89HDF\r\n\x1a\n" {
        return Err(bad("not an HDF5 file"));
    }
    if !(2..=3).contains(&sb[8]) || sb[9] != 8 || sb[10] != 8 {
        return Err(bad(format!(
            "unsupported superblock v{} ({}/{} byte offsets)",
            sb[8], sb[9], sb[10]
        )));
    }
    u64_at(&sb, 36)
}

/// Hard links of the root group, `(name, object header address)`.
pub fn root_links(r: &mut dyn ReadAt) -> io::Result<Vec<(String, u64)>> {
    let root = root_address(r)?;
    let mut links = Vec::new();
    for (t, b) in header_messages(r, root)? {
        if t != 0x06 {
            continue;
        }
        let flags = *b.get(1).ok_or_else(|| bad("short link"))?;
        let mut i = 2;
        if flags & 0x08 != 0 {
            if b[i] != 0 {
                continue; // soft or external link
            }
            i += 1;
        }
        if flags & 0x04 != 0 {
            i += 8;
        }
        if flags & 0x10 != 0 {
            i += 1;
        }
        let len_size = 1usize << (flags & 3);
        let n = uint_at(&b, i, len_size)? as usize;
        i += len_size;
        let name =
            String::from_utf8_lossy(b.get(i..i + n).ok_or_else(|| bad("short name"))?).into_owned();
        links.push((name, u64_at(&b, i + n)?));
    }
    Ok(links)
}

/// The dataset named `name` in the root group.
pub fn dataset(r: &mut dyn ReadAt, name: &str) -> io::Result<Dataset> {
    let addr = root_links(r)?
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, a)| a)
        .ok_or_else(|| bad(format!("dataset {name:?} not in the root group")))?;
    let (mut dims, mut element_size, mut float_le, mut filtered, mut layout) =
        (None, 0, false, false, None);
    for (t, b) in header_messages(r, addr)? {
        match t {
            0x01 => {
                if b[0] != 2 {
                    return Err(bad(format!("dataspace message v{}", b[0])));
                }
                let rank = b[1] as usize;
                let mut d = Vec::with_capacity(rank);
                for k in 0..rank {
                    d.push(u64_at(&b, 4 + 8 * k)?);
                }
                dims = Some(d);
            }
            0x03 => {
                // Class 1 is floating point; bit 0 of the class bits is big-endian.
                float_le = b[0] & 0x0f == 1 && b[1] & 0x01 == 0;
                element_size = u32_at(&b, 4)?;
            }
            0x0b => filtered = true,
            0x08 => {
                if b[0] != 3 {
                    return Err(bad(format!("layout message v{}", b[0])));
                }
                layout = Some(match b[1] {
                    1 => Layout::Contiguous {
                        address: u64_at(&b, 2)?,
                        size: u64_at(&b, 10)?,
                    },
                    2 => {
                        let n = b[2] as usize;
                        let btree = u64_at(&b, 3)?;
                        let mut chunk = Vec::with_capacity(n);
                        for k in 0..n {
                            chunk.push(u64::from(u32_at(&b, 11 + 4 * k)?));
                        }
                        chunk.pop(); // the trailing dimension is the element size
                        Layout::Chunked { btree, chunk }
                    }
                    c => return Err(bad(format!("layout class {c}"))),
                });
            }
            _ => {}
        }
    }
    Ok(Dataset {
        dims: dims.ok_or_else(|| bad("no dataspace"))?,
        element_size,
        float_le,
        filtered,
        layout: layout.ok_or_else(|| bad("no layout"))?,
    })
}

/// Every chunk of a rank-3 chunked dataset, sorted by offset.
pub fn chunks(r: &mut dyn ReadAt, btree: u64) -> io::Result<Vec<Chunk>> {
    const RANK: usize = 3;
    let key = 8 + 8 * (RANK + 1);
    let mut out = Vec::new();
    let mut stack = vec![btree];
    let mut visited = 0usize;
    while let Some(node) = stack.pop() {
        visited += 1;
        if visited > 100_000 {
            return Err(bad("chunk B-tree too large"));
        }
        let head = r.read_at(node, 24)?;
        if &head[..4] != b"TREE" || head[4] != 1 {
            return Err(bad(format!("no chunk B-tree node at {node}")));
        }
        let level = head[5];
        let used = u16_at(&head, 6)? as usize;
        let body = r.read_at(node + 24, used * (key + 8) + key)?;
        for e in 0..used {
            let k = e * (key + 8);
            let child = u64_at(&body, k + key)?;
            if level > 0 {
                stack.push(child);
                continue;
            }
            let mut offset = [0u64; RANK];
            for (d, o) in offset.iter_mut().enumerate() {
                *o = u64_at(&body, k + 8 + 8 * d)?;
            }
            out.push(Chunk {
                offset,
                address: child,
                size: u32_at(&body, k)?,
                filter_mask: u32_at(&body, k + 4)?,
            });
        }
    }
    out.sort_by_key(|c| c.offset);
    Ok(out)
}

/// A contiguous little-endian `f32` vector dataset (a coordinate axis).
pub fn read_f32_vector(r: &mut dyn ReadAt, ds: &Dataset) -> io::Result<Vec<f32>> {
    let Layout::Contiguous { address, size } = ds.layout else {
        return Err(bad("axis is not contiguous"));
    };
    if !ds.float_le || ds.element_size != 4 || ds.dims.len() != 1 || size != ds.dims[0] * 4 {
        return Err(bad("axis is not a float32 vector"));
    }
    let b = r.read_at(address, size as usize)?;
    Ok(b.as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect())
}
