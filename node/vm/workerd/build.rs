fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema_file = "workerd.capnp";

    println!("cargo:rerun-if-changed={}", schema_file);

    capnpc::CompilerCommand::new()
        .file(schema_file)
        .output_path(std::env::var("OUT_DIR").unwrap())
        .run()?;

    Ok(())
}
