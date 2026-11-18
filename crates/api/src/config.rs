//! Server settings, read from the environment.
//!
//! | Variable       | Default        | Meaning                                                    |
//! | -------------- | -------------- | ---------------------------------------------------------- |
//! | `AX_BIND`      | `0.0.0.0:8080` | Address to listen on                                       |
//! | `AX_BASE_PATH` | `/`            | Path the contract is mounted at (see below)                |
//! | `DATABASE_URL` | unset          | PostgreSQL URL; unset runs without a database (dev, tests) |
//!
//! The frontend's nginx config proxies `/api/` to this server with a trailing
//! slash, which strips the prefix, so the default mounts the contract at `/`.
//! Set `AX_BASE_PATH=/api` when a proxy forwards the prefix unchanged.

use std::fmt;
use std::net::SocketAddr;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    /// `/` or a path without a trailing slash, e.g. `/api`.
    pub base_path: String,
    pub database_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads the process environment.
    ///
    /// # Errors
    ///
    /// Returns an error naming the variable when a value is malformed.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads settings through `get`, so tests need not touch the environment.
    ///
    /// # Errors
    ///
    /// Returns an error naming the variable when a value is malformed.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let non_empty = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());

        let bind = non_empty("AX_BIND").unwrap_or_else(|| "0.0.0.0:8080".to_owned());
        let bind = bind.parse().map_err(|_| ConfigError(format!("AX_BIND must be host:port, got {bind:?}")))?;

        let base_path = non_empty("AX_BASE_PATH").unwrap_or_else(|| "/".to_owned());
        if !base_path.starts_with('/') || base_path.contains("//") {
            return Err(ConfigError(format!("AX_BASE_PATH must start with one '/', got {base_path:?}")));
        }
        let base_path = if base_path == "/" { base_path } else { base_path.trim_end_matches('/').to_owned() };

        Ok(Self { bind, base_path, database_url: non_empty("DATABASE_URL") })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
        Config::from_lookup(|name| vars.get(name).cloned())
    }

    #[test]
    fn defaults() {
        let c = config(&[]).unwrap();
        assert_eq!(c.bind.to_string(), "0.0.0.0:8080");
        assert_eq!(c.base_path, "/");
        assert_eq!(c.database_url, None);
    }

    #[test]
    fn base_path_loses_its_trailing_slash() {
        assert_eq!(config(&[("AX_BASE_PATH", "/api/")]).unwrap().base_path, "/api");
    }

    #[test]
    fn rejects_bad_values() {
        assert!(config(&[("AX_BIND", "8080")]).is_err());
        assert!(config(&[("AX_BASE_PATH", "api")]).is_err());
        assert!(config(&[("AX_BASE_PATH", "//api")]).is_err());
    }

    #[test]
    fn blank_values_count_as_unset() {
        let c = config(&[("DATABASE_URL", "  "), ("AX_BIND", "")]).unwrap();
        assert_eq!(c.database_url, None);
        assert_eq!(c.bind.to_string(), "0.0.0.0:8080");
    }
}
