//! tonic-build codegen for the PingoGate gRPC contract.
//!
//! Mirrors `pingogate-core/build.rs`: generates server + client stubs from
//! `proto/pingogate.proto` into `OUT_DIR/pingogate.rs`. The keyvault crate owns
//! the `KeyVaultService` server impl, so it must generate the proto traits it
//! implements. `core-rs/keyvault/` is one level under `core-rs/`, so the proto
//! root resolves identically to `pingogate-core` (both `../../proto`).

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Repo root is two levels up from `core-rs/keyvault/`.
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
