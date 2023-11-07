use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .out_dir("./src")
        .compile(&["proto/rustyrmm-endpoint.proto"], &["./proto", "/usr/local/include/google"])?;

    Ok(())
}