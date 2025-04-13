fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile_protos(
        &[
            "proto/daemon.proto",
            "proto/enclave.proto",
            "proto/interface.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
