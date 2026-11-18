use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    // Vendored protoc and well-known-type includes: no system protoc needed locally or in CI.
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);
    let well_known = protoc_bin_vendored::include_path()?;

    // compile_protos takes one path type for both lists, so use PathBuf throughout.
    let protos = ["common", "agent", "operator"]
        .map(|p| PathBuf::from(format!("proto/rustyrmm/{p}/v1/{p}.proto")));
    tonic_prost_build::configure()
        .compile_protos(&protos, &[PathBuf::from("proto"), well_known])?;
    println!("cargo:rerun-if-changed=proto");
    Ok(())
}
