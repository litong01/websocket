//! Configuration from environment.

pub struct Config {
    pub host: String,
    pub port: u16,
    pub kinde_domain: String,
    pub kinde_audience: Option<String>,
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
        }
    }
}
