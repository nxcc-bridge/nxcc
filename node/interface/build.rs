fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile_protos(
        &[
            "proto/interface.proto",
            "proto/daemon.proto",
            "proto/enclave.proto",
            "proto/vm.proto",
        ],
        &["proto"],
    )?;
    Ok(())
}
