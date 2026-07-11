use skid_monitor_server::MigrationConfig;
use skid_monitor_server::store::{PgSignalStore, PgStoreOptions};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = MigrationConfig::from_env().unwrap_or_else(|error| {
        eprintln!("skid-monitor migration configuration error: {error}");
        std::process::exit(2);
    });
    let result = async {
        let store = PgSignalStore::connect(
            &config.database.url,
            PgStoreOptions {
                max_connections: config.database.max_connections,
                ..PgStoreOptions::default()
            },
        )
        .await?;
        store.migrate().await
    }
    .await;
    if let Err(error) = result {
        eprintln!("skid-monitor migration failed: {error}");
        std::process::exit(1);
    }
}
