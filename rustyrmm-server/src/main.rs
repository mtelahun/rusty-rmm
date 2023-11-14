use rustyrmm_server::startup;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let settings = startup::get_settings();
    let addr = startup::build_address(format!(
        "{}:{}",
        settings.listen_address, settings.listen_port
    ))?;
    startup::serve(addr, settings.database.database_url()).await;

    Ok(())
}
