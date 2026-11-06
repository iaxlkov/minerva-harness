#[tokio::main]
async fn main() {
    let result = match minerva_gateway::config::GatewayConfig::from_env() {
        Ok(config) => minerva_gateway::start(config).await,
        Err(_) => Err(minerva_gateway::StartupError),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
