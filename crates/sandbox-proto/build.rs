fn main() -> Result<(), Box<dyn std::error::Error>> {
    // prost-build resolves the compiler from $PROTOC before falling back to
    // `protoc` on PATH. Point it at the vendored binary so nothing here needs a
    // system protobuf-compiler: the CI container jobs run on ARC runners whose
    // NetworkPolicy blocks plain-HTTP apt, and a fresh clone builds with just
    // `cargo build`.
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(&["proto/sandbox/v1/sandbox.proto"], &["proto"])?;
    Ok(())
}
