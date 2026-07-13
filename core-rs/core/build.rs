// core-rs/core/build.rs
fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure()
        .build_server(false)
        .build_client(true)
        .compile_protos(&["../../proto/pingogate.proto"], &["../../proto"])?;
    Ok(())
}
