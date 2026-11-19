//! Server settings, read from the environment.
//!
//! | Variable          | Default        | Meaning                                                    |
//! | ----------------- | -------------- | ---------------------------------------------------------- |
//! | `AX_BIND`         | `0.0.0.0:8080` | Address to listen on                                       |
//! | `AX_BASE_PATH`    | `/`            | Path the contract is mounted at (see below)                |
//! | `DATABASE_URL`    | unset          | PostgreSQL URL; unset runs without a database (dev, tests) |
//! | `AX_SESSION_IDLE` | `24h`          | A sign-in ends after this long unused (`s`, `m`, `h`, `d`) |
//! | `AX_SESSION_MAX`  | `7d`           | ...and this long after signing in, however much it is used |
//! | `AX_SETUP_LINK_TTL` | `72h`        | How long a one-time setup link works                       |
//! | `AX_TIME_ZONE`    | `Asia/Seoul`   | The business calendar: "today", due dates, monthly totals  |
//! | `AX_AI_PROVIDER`  | `stub`         | Who writes reports and runs work (`stub` until one is wired)|
//! | `AX_SCOPE_CHECK`  | `true`         | The requester confirms the AI's reading before review      |
//! | `AX_UPLOAD_DIR`   | `uploads`      | Where uploaded attachments are kept (created if missing)   |
//! | `AX_PUBLIC_PATH`  | `/api`         | Where the browser reaches this server, for attachment links |
//!
//! The frontend's nginx config proxies `/api/` to this server with a trailing
//! slash, which strips the prefix, so the default mounts the contract at `/`.
//! Set `AX_BASE_PATH=/api` when a proxy forwards the prefix unchanged.

use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    /// `/` or a path without a trailing slash, e.g. `/api`.
    pub base_path: String,
    pub database_url: Option<String>,
    pub sessions: SessionPolicy,
    pub setup_links: SetupLinkPolicy,
    pub time_zone: BusinessTimeZone,
    pub ai: AiSettings,
    /// `AX_UPLOAD_DIR`: one file per attachment.
    pub upload_dir: PathBuf,
    /// `AX_PUBLIC_PATH`: the path the browser puts before this server's
    /// routes (`/api` behind the frontend's nginx), without a trailing slash
    /// (`/` stays `/`). Attachment links start with it.
    pub public_path: String,
}

/// How the AI's jobs run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiSettings {
    /// `stub`: writes an honest "no AI provider" report and work result, so
    /// the lifecycle can be used end to end before a provider is wired.
    pub provider: String,
    /// The frontend's `appConfig.workflow.scopeCheck`: after a feasible or
    /// conditional report the requester confirms the AI's reading of the
    /// request (`scope_check`) before a developer reviews it.
    pub scope_check: bool,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { provider: "stub".to_owned(), scope_check: true }
    }
}

/// The time zone of the business calendar (IANA name): what "today" is for
/// desired and release dates, delays and monthly totals. The frontend's
/// `appConfig.organization.timeZone`; both must name the same zone. Dates are
/// computed by PostgreSQL, which checks the name when the server starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusinessTimeZone(pub String);

impl Default for BusinessTimeZone {
    fn default() -> Self {
        Self("Asia/Seoul".to_owned())
    }
}

/// How long a one-time setup link (to set or reset a password) works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetupLinkPolicy {
    pub ttl: Duration,
}

impl Default for SetupLinkPolicy {
    fn default() -> Self {
        Self { ttl: Duration::from_secs(72 * HOUR) }
    }
}

/// How long a sign-in lasts. Each use moves the end `idle` ahead, but never
/// past `max` after sign-in. There is no refresh: the frontend signs in again
/// after a `401`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionPolicy {
    pub idle: Duration,
    pub max: Duration,
}

impl Default for SessionPolicy {
    fn default() -> Self {
        Self { idle: Duration::from_secs(24 * HOUR), max: Duration::from_secs(7 * DAY) }
    }
}

const MINUTE: u64 = 60;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

/// `90s`, `15m`, `24h`, `7d`.
fn parse_duration(value: &str) -> Option<Duration> {
    let split = value.find(|c: char| !c.is_ascii_digit())?;
    let (number, unit) = value.split_at(split);
    let unit = match unit {
        "s" => 1,
        "m" => MINUTE,
        "h" => HOUR,
        "d" => DAY,
        _ => return None,
    };
    let seconds = number.parse::<u64>().ok()?.checked_mul(unit)?;
    (seconds > 0).then(|| Duration::from_secs(seconds))
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

        let duration = |name: &str, default: Duration| match non_empty(name) {
            None => Ok(default),
            Some(value) => parse_duration(&value)
                .ok_or_else(|| ConfigError(format!("{name} must be a duration such as 30m, 24h or 7d, got {value:?}"))),
        };
        let defaults = SessionPolicy::default();
        let sessions = SessionPolicy {
            idle: duration("AX_SESSION_IDLE", defaults.idle)?,
            max: duration("AX_SESSION_MAX", defaults.max)?,
        };
        if sessions.idle > sessions.max {
            return Err(ConfigError("AX_SESSION_IDLE must not be longer than AX_SESSION_MAX".to_owned()));
        }

        let setup_links = SetupLinkPolicy { ttl: duration("AX_SETUP_LINK_TTL", SetupLinkPolicy::default().ttl)? };

        let time_zone = non_empty("AX_TIME_ZONE").map_or_else(BusinessTimeZone::default, BusinessTimeZone);
        let provider = non_empty("AX_AI_PROVIDER").unwrap_or_else(|| "stub".to_owned());
        if provider != "stub" {
            return Err(ConfigError(format!(
                "AX_AI_PROVIDER must be stub (the only provider so far), got {provider:?}"
            )));
        }
        let scope_check = match non_empty("AX_SCOPE_CHECK").as_deref() {
            None | Some("true") => true,
            Some("false") => false,
            Some(other) => return Err(ConfigError(format!("AX_SCOPE_CHECK must be true or false, got {other:?}"))),
        };
        let ai = AiSettings { provider, scope_check };

        let upload_dir = PathBuf::from(non_empty("AX_UPLOAD_DIR").unwrap_or_else(|| "uploads".to_owned()));
        let public_path = non_empty("AX_PUBLIC_PATH").unwrap_or_else(|| "/api".to_owned());
        if !public_path.starts_with('/') || public_path.contains("//") {
            return Err(ConfigError(format!("AX_PUBLIC_PATH must start with one '/', got {public_path:?}")));
        }
        let public_path = if public_path == "/" { public_path } else { public_path.trim_end_matches('/').to_owned() };

        Ok(Self {
            bind,
            base_path,
            database_url: non_empty("DATABASE_URL"),
            sessions,
            setup_links,
            time_zone,
            ai,
            upload_dir,
            public_path,
        })
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
        assert_eq!(c.sessions.idle, Duration::from_secs(24 * 3600));
        assert_eq!(c.sessions.max, Duration::from_secs(7 * 24 * 3600));
        assert_eq!(c.setup_links.ttl, Duration::from_secs(72 * 3600));
        assert_eq!(c.time_zone.0, "Asia/Seoul");
        assert_eq!(c.ai, AiSettings { provider: "stub".into(), scope_check: true });
        assert!(!config(&[("AX_SCOPE_CHECK", "false")]).unwrap().ai.scope_check);
        assert!(config(&[("AX_SCOPE_CHECK", "no")]).is_err());
        assert!(config(&[("AX_AI_PROVIDER", "magic")]).is_err());
        assert_eq!(config(&[("AX_TIME_ZONE", "Europe/Berlin")]).unwrap().time_zone.0, "Europe/Berlin");
        assert_eq!(config(&[("AX_SETUP_LINK_TTL", "2d")]).unwrap().setup_links.ttl, Duration::from_secs(48 * 3600));
        assert!(config(&[("AX_SETUP_LINK_TTL", "soon")]).is_err());
    }

    #[test]
    fn session_lifetimes() {
        let c = config(&[("AX_SESSION_IDLE", "30m"), ("AX_SESSION_MAX", "12h")]).unwrap();
        assert_eq!(c.sessions.idle, Duration::from_secs(30 * 60));
        assert_eq!(c.sessions.max, Duration::from_secs(12 * 3600));
        assert_eq!(config(&[("AX_SESSION_IDLE", "90s")]).unwrap().sessions.idle, Duration::from_secs(90));

        for bad in ["0h", "24", "h", "1w", "-1h", "1.5h", "99999999999999999999d"] {
            assert!(config(&[("AX_SESSION_IDLE", bad)]).is_err(), "{bad}");
        }
        assert!(config(&[("AX_SESSION_IDLE", "8d")]).is_err(), "idle longer than the default max");
    }

    #[test]
    fn base_path_loses_its_trailing_slash() {
        assert_eq!(config(&[("AX_BASE_PATH", "/api/")]).unwrap().base_path, "/api");
    }

    #[test]
    fn uploads_and_their_links() {
        let c = config(&[]).unwrap();
        assert_eq!((c.upload_dir, c.public_path.as_str()), (PathBuf::from("uploads"), "/api"));
        let c = config(&[("AX_UPLOAD_DIR", "/var/lib/ax/uploads"), ("AX_PUBLIC_PATH", "/portal/api/")]).unwrap();
        assert_eq!((c.upload_dir, c.public_path.as_str()), (PathBuf::from("/var/lib/ax/uploads"), "/portal/api"));
        assert_eq!(config(&[("AX_PUBLIC_PATH", "/")]).unwrap().public_path, "/");
        for bad in ["api", "//api", "https://example.com/api"] {
            assert!(config(&[("AX_PUBLIC_PATH", bad)]).is_err(), "{bad}");
        }
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
