//! Exposes the Temporal API descriptor set (built by `temporalio-protos`) to this crate's tests as
//! `TEMPORAL_PROTO_DESCRIPTORS`. Only the offline replay gate (`tests/replay.rs`) reads it, to parse
//! proto3-JSON workflow histories.

fn main() {
    println!("cargo:rerun-if-env-changed=DEP_TEMPORALIO_PROTOS_DESCRIPTOR_PATH");
    if let Ok(path) = std::env::var("DEP_TEMPORALIO_PROTOS_DESCRIPTOR_PATH") {
        println!("cargo:rustc-env=TEMPORAL_PROTO_DESCRIPTORS={path}");
    }
}
