//! Configuration from environment.

pub struct Config {
    pub host: String,
    pub port: u16,
    pub kinde_domain: String,
    pub kinde_audience: Option<String>,
    /// Idle timeout in seconds; connection is closed after this long with no activity. Default 7200 (2 hours).
    pub idle_timeout_secs: u64,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            host: std::env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            kinde_domain: std::env::var("KINDE_DOMAIN")
                .expect("KINDE_DOMAIN must be set (e.g. myapp for myapp.kinde.com)"),
            kinde_audience: std::env::var("KINDE_AUDIENCE").ok(),
            idle_timeout_secs: std::env::var("IDLE_TIMEOUT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(7200),
        }
    }
}
