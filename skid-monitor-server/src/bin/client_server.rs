use skid_monitor_server::{ClientServerConfig, api};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = ClientServerConfig::from_env().unwrap_or_else(|error| {
        eprintln!("skid-monitor-client-server configuration error: {error}");
        std::process::exit(2);
    });
    if let Err(error) = api::serve(config).await {
        eprintln!("skid-monitor-client-server failed: {error}");
        std::process::exit(1);
    }
}
