//! tonic-build codegen for the PingoGate gRPC contract.
//!
//! Generates server + client stubs from `proto/pingogate.proto` into
//! `OUT_DIR/pingogate.rs`. The binary runs the server (platform mode) and
//! also embeds the client types for future integration tests. Protobuf fields
//! are kept as their raw snake_case names; the module is re-exported from
//! [`crate::grpc`] via `include_proto!`.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Repo root is two levels up from `core-rs/pingogate-core/`.
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let proto_root = manifest_dir
        .join("..")
        .join("..")
        .join("proto")
        .canonicalize()?;
    let proto_path = proto_root.join("pingogate.proto");

    println!("cargo:rerun-if-changed={}", proto_path.display());

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&[&proto_path], &[&proto_root])?;

    Ok(())
}
