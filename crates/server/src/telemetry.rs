//! Logging setup. JSON in production, human-readable in development.

use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::Config;

pub fn init(config: &Config) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,carelibre_server=debug,tower_http=info,sqlx=warn")
    });

    let registry = tracing_subscriber::registry().with(filter);

    if config.environment.is_production() {
        registry.with(fmt::layer().json()).init();
    } else {
        registry.with(fmt::layer()).init();
    }
}
