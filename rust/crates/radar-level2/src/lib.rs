#![forbid(unsafe_code)]
pub mod artifact;
pub mod catalogue;
pub mod decode;
pub mod feed;
pub mod health;
pub mod leases;
pub mod polar;
pub mod tile;
pub mod tracer;

pub const MIB: usize = 1024 * 1024;
pub const COMPRESSED_LIMIT: usize = 32 * MIB;
pub const RECORD_LIMIT: usize = 8 * MIB;
pub const EXPANDED_LIMIT: usize = 256 * MIB;
pub const RADIAL_LIMIT: usize = 2048;
pub const GATE_LIMIT: usize = 4096;
pub const SWEEP_LIMIT: usize = 32;
pub const HOT_LIMIT: usize = 1024 * MIB;
pub const JOB_RESERVATION: usize = HOT_LIMIT / 2;
pub const ARRAY_LIMIT: usize =
    (JOB_RESERVATION - COMPRESSED_LIMIT - 2 * RECORD_LIMIT - 16 * MIB) / 3;
pub const RENDER_LIMIT: usize = 512 * MIB;
pub const RENDER_JOBS: usize = 2;
pub const OBJECT_LIMIT: i64 = 10 * 1024 * 1024 * 1024;

pub fn checksum(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
pub(crate) mod testdata;
