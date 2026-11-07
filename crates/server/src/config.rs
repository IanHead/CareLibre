//! Runtime configuration, loaded from the process environment.

use std::{env, net::SocketAddr, time::Duration};

use anyhow::{bail, Context};

/// Deployment environment. Controls logging format and strictness of config
/// validation (e.g. secrets and CORS defaults are enforced in production).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Development,
    Production,
}

impl Environment {
    #[must_use]
    pub const fn is_production(self) -> bool {
        matches!(self, Self::Production)
    }
}

/// Fully resolved server configuration.
///
/// Deliberately does not implement `Debug`: it holds secrets.
#[derive(Clone)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub environment: Environment,
    pub database_url: String,
    pub database_max_connections: u32,
    pub run_migrations: bool,
    pub jwt_secret: String,
    pub access_token_ttl: Duration,
    pub refresh_token_ttl: Duration,
    pub cors_allowed_origins: Vec<String>,
}

impl Config {
    /// Load configuration from environment variables, applying development
    /// defaults where it is safe to do so.
    pub fn from_env() -> anyhow::Result<Self> {
        let environment = match env::var("APP_ENV")
            .unwrap_or_else(|_| "development".to_owned())
            .to_ascii_lowercase()
            .as_str()
        {
            "production" | "prod" => Environment::Production,
            "development" | "dev" | "test" => Environment::Development,
            other => bail!("APP_ENV must be one of development, production (got {other:?})"),
        };

        let bind_addr = env::var("BIND_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
            .parse()
            .context("BIND_ADDR must be a valid host:port socket address")?;

        let database_url = env::var("DATABASE_URL")
            .context("DATABASE_URL is required (e.g. postgres://user:pass@host:5432/db)")?;

        let database_max_connections = parse_u32("DB_MAX_CONNECTIONS", 10)?;
        let run_migrations = parse_bool("RUN_MIGRATIONS", true)?;

        let jwt_secret = match env::var("JWT_SECRET") {
            Ok(secret) => secret,
            Err(_) if environment.is_production() => {
                bail!("JWT_SECRET is required when APP_ENV=production")
            }
            Err(_) => {
                // Development convenience: an ephemeral secret means tokens are
                // invalidated on every restart, which is the safe behaviour.
                let generated = generate_ephemeral_secret();
                tracing::warn!(
                    "JWT_SECRET not set; generated an ephemeral development secret. \
                     Access tokens will not survive a restart."
                );
                generated
            }
        };

        if jwt_secret.len() < 32 {
            bail!("JWT_SECRET must be at least 32 characters long");
        }

        let access_token_ttl = Duration::from_secs(parse_u64("ACCESS_TOKEN_TTL_SECS", 900)?);
        let refresh_token_ttl =
            Duration::from_secs(parse_u64("REFRESH_TOKEN_TTL_SECS", 60 * 60 * 24 * 30)?);

        if access_token_ttl.is_zero() {
            bail!("ACCESS_TOKEN_TTL_SECS must be greater than zero");
        }
        if refresh_token_ttl <= access_token_ttl {
            bail!("REFRESH_TOKEN_TTL_SECS must be greater than ACCESS_TOKEN_TTL_SECS");
        }

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        if environment.is_production() && cors_allowed_origins.iter().any(|o| o == "*") {
            bail!("CORS_ALLOWED_ORIGINS may not contain '*' when APP_ENV=production");
        }

        Ok(Self {
            bind_addr,
            environment,
            database_url,
            database_max_connections,
            run_migrations,
            jwt_secret,
            access_token_ttl,
            refresh_token_ttl,
            cors_allowed_origins,
        })
    }
}

fn parse_u64(key: &str, default: u64) -> anyhow::Result<u64> {
    match env::var(key) {
        Ok(raw) => raw
            .parse()
            .with_context(|| format!("{key} must be an unsigned integer, got {raw:?}")),
        Err(_) => Ok(default),
    }
}

fn parse_u32(key: &str, default: u32) -> anyhow::Result<u32> {
    let value = parse_u64(key, u64::from(default))?;
    u32::try_from(value).with_context(|| format!("{key} is out of range: {value}"))
}

fn parse_bool(key: &str, default: bool) -> anyhow::Result<bool> {
    match env::var(key) {
        Ok(raw) => match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => bail!("{key} must be a boolean, got {other:?}"),
        },
        Err(_) => Ok(default),
    }
}

fn generate_ephemeral_secret() -> String {
    use rand::RngCore;

    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}
