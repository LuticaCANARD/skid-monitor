use skid_monitor_server::{IngressConfig, ingress};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = IngressConfig::from_env().unwrap_or_else(|error| {
        eprintln!("skid-monitor-ingress configuration error: {error}");
        std::process::exit(2);
    });
    if let Err(error) = ingress::serve(config).await {
        eprintln!("skid-monitor-ingress failed: {error}");
        std::process::exit(1);
    }
}
