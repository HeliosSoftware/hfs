use clap::Parser;

/// Configuration for the Helios Terminology Service.
#[derive(Parser, Debug, Clone)]
#[command(
    name = "hts",
    about = "Helios Terminology Service — FHIR Terminology Operations",
    version
)]
pub struct HtsConfig {
    /// Server port
    #[arg(long, env = "HTS_SERVER_PORT", default_value = "8090")]
    pub port: u16,

    /// Server host to bind
    #[arg(long, env = "HTS_SERVER_HOST", default_value = "127.0.0.1")]
    pub host: String,

    /// Log level (error, warn, info, debug, trace)
    #[arg(long, env = "HTS_LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Database URL (SQLite file path or PostgreSQL connection string)
    #[arg(long, env = "HTS_DATABASE_URL", default_value = "./data/hts.db")]
    pub database_url: String,

    /// Storage backend (sqlite | postgres)
    #[arg(long, env = "HTS_STORAGE_BACKEND", default_value = "sqlite")]
    pub storage_backend: String,

    /// Enable CORS
    #[arg(long, env = "HTS_ENABLE_CORS", default_value = "true")]
    pub enable_cors: bool,

    /// Allowed CORS origins (comma-separated)
    #[arg(long, env = "HTS_CORS_ORIGINS", default_value = "*")]
    pub cors_origins: String,

    /// Maximum number of codes allowed in a single ValueSet expansion.
    /// Requests that would exceed this limit receive HTTP 422 with issue
    /// code `too-costly`.
    #[arg(long, env = "HTS_MAX_EXPANSION_SIZE", default_value = "10000")]
    pub max_expansion_size: u32,
}

impl HtsConfig {
    /// Returns the socket address string for binding.
    pub fn socket_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl Default for HtsConfig {
    fn default() -> Self {
        Self {
            port: 8090,
            host: "127.0.0.1".into(),
            log_level: "info".into(),
            database_url: "./data/hts.db".into(),
            storage_backend: "sqlite".into(),
            enable_cors: true,
            cors_origins: "*".into(),
            max_expansion_size: 10_000,
        }
    }
}
