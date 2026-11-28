fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Point prost-build at the vendored protoc so nothing here needs a system
    // protobuf-compiler: the CI container jobs run on ARC runners whose
    // NetworkPolicy blocks plain-HTTP apt, and a fresh clone builds with just
    // `cargo build`.
    let mut config = tonic_prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);

    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_with_config(config, &["proto/sandbox/v1/sandbox.proto"], &["proto"])?;
    Ok(())
}
