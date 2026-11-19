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
//! | `AX_LOGIN_ATTEMPTS` | `5`          | Failed sign-ins per email before a lockout                 |
//! | `AX_LOGIN_LOCKOUT`  | `15m`        | The window they are counted in, and the lockout's length   |
//! | `AX_TIME_ZONE`    | `Asia/Seoul`   | The business calendar: "today", due dates, monthly totals  |
//! | `AX_AI_PROVIDER`  | `stub`         | Who writes reports: `stub`, or `anthropic` (Claude)        |
//! | `AX_SCOPE_CHECK`  | `true`         | The requester confirms the AI's reading before review      |
//! | `AX_RUNNER_TOKEN` | unset          | Lets a runner (`ax-api runner`) take the AI jobs (see below) |
//! | `AX_ANTHROPIC_API_KEY` | unset     | Claude Platform key for every AI task (see below)          |
//! | `AX_ANTHROPIC_API_KEY_PRECHECK`, `_REPORT`, `_WORK` | unset | A key for one task, used before the one above |
//! | `AX_DRAFTS_PER_ACCOUNT` | `20`     | How many saved drafts one person may keep                  |
//! | `AX_UPLOAD_DIR`   | `uploads`      | Where uploaded attachments are kept (created if missing)   |
//! | `AX_PUBLIC_PATH`  | `/api`         | Where the browser reaches this server, for attachment links |
//!
//! The frontend's nginx config proxies `/api/` to this server with a trailing
//! slash, which strips the prefix, so the default mounts the contract at `/`.
//! Set `AX_BASE_PATH=/api` when a proxy forwards the prefix unchanged.
//!
//! Provider keys come only from here, never from the API: the console shows a
//! provider as connected with the last four characters of its keys, and
//! `setProviderKey` / `removeProviderKey` answer `403`. A key per task lets an
//! organisation see each task's spending on its own key.

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
    pub login: LoginPolicy,
    pub time_zone: BusinessTimeZone,
    pub ai: AiSettings,
    pub drafts: DraftPolicy,
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
    /// the lifecycle can be used end to end without one. `anthropic`: Claude
    /// writes the report (see `ai::claude`); work runs still wait for a
    /// coding agent.
    pub provider: String,
    /// The frontend's `appConfig.workflow.scopeCheck`: after a feasible or
    /// conditional report the requester confirms the AI's reading of the
    /// request (`scope_check`) before a developer reviews it.
    pub scope_check: bool,
    /// Anthropic (Claude Platform) keys.
    pub anthropic: TaskKeys,
    /// `AX_RUNNER_TOKEN`: when set, a runner takes the report and work jobs
    /// over HTTP (`ai::runner`) and the server runs none itself.
    pub runner_token: Option<Secret>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self { provider: "stub".to_owned(), scope_check: true, anthropic: TaskKeys::default(), runner_token: None }
    }
}

/// A secret from the environment. Its `Debug` output never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The secret itself, for the one place that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// The last four characters, which the console shows.
    pub fn hint(&self) -> String {
        let tail: Vec<char> = self.0.chars().rev().take(4).collect();
        tail.into_iter().rev().collect()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// One provider's keys: one for every task, and optionally one per task,
/// which wins for that task.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskKeys {
    pub default: Option<Secret>,
    pub precheck: Option<Secret>,
    pub report: Option<Secret>,
    pub work: Option<Secret>,
}

impl TaskKeys {
    /// The key for `task` (`precheck`, `report` or `work`).
    pub fn for_task(&self, task: &str) -> Option<&Secret> {
        let own = match task {
            "precheck" => self.precheck.as_ref(),
            "report" => self.report.as_ref(),
            "work" => self.work.as_ref(),
            _ => None,
        };
        own.or(self.default.as_ref())
    }

    /// Whether any key is set: the provider counts as connected.
    pub fn connected(&self) -> bool {
        [&self.default, &self.precheck, &self.report, &self.work].iter().any(|k| k.is_some())
    }

    /// The last four characters of each distinct key, in the order default,
    /// precheck, report, work; `None` without keys.
    pub fn hint(&self) -> Option<String> {
        let mut hints: Vec<String> = Vec::new();
        for key in [&self.default, &self.precheck, &self.report, &self.work].into_iter().flatten() {
            let hint = key.hint();
            if !hints.contains(&hint) {
                hints.push(hint);
            }
        }
        (!hints.is_empty()).then(|| hints.join(", "))
    }
}

/// Saved drafts: how many one person may keep (the frontend's template
/// setting `drafts.maxPerAccount`). One more is `409 limit_reached`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DraftPolicy {
    pub per_account: u32,
}

impl Default for DraftPolicy {
    fn default() -> Self {
        Self { per_account: 20 }
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

/// Password guessing: after `attempts` failed sign-ins for one email within
/// `lockout`, sign-ins for it are refused for `lockout` (see `auth::throttle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoginPolicy {
    pub attempts: u32,
    pub lockout: Duration,
}

impl Default for LoginPolicy {
    fn default() -> Self {
        Self { attempts: 5, lockout: Duration::from_secs(15 * MINUTE) }
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

/// The runner's settings (`ax-api runner`): where the server is, how to reach
/// GitHub and Claude, where to work.
///
/// | Variable | Default | Meaning |
/// | --- | --- | --- |
/// | `AX_RUNNER_SERVER` | required | The API as the runner reaches it, e.g. `http://ax-backend:8080` |
/// | `AX_RUNNER_TOKEN` | required | The same token as the server's |
/// | `AX_GITHUB_TOKEN` | required | Pushes the run's branch and opens its pull request; give it no right to merge |
/// | `AX_GITHUB_API` | `https://api.github.com` | GitHub's REST API (GitHub Enterprise: `https://HOST/api/v3`) |
/// | `AX_ANTHROPIC_API_KEY` (and `_REPORT`, `_WORK`) | required | Claude keys, as on the server |
/// | `AX_RUNNER_WORKDIR` | temp dir | Where repositories are cloned, one directory per job |
/// | `AX_RUNNER_TIMEOUT` | `30m` | How long one agent run may take |
/// | `AX_CLAUDE_BIN` | `claude` | The Claude Code command |
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    pub server: String,
    pub token: Secret,
    pub github_token: Secret,
    pub github_api: String,
    pub anthropic: TaskKeys,
    pub workdir: PathBuf,
    pub timeout: Duration,
    pub claude: String,
    /// Extra `git -c` settings for every git command (tests point GitHub at
    /// a local repository with `url.<path>.insteadOf`).
    pub git_config: Vec<(String, String)>,
    /// How often to ask for work when there is none, and to renew a claim.
    pub poll: Duration,
    pub heartbeat: Duration,
}

impl RunnerConfig {
    /// Reads the process environment.
    ///
    /// # Errors
    ///
    /// A missing or malformed variable, by name.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// # Errors
    ///
    /// A missing or malformed variable, by name.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let non_empty = |name: &str| get(name).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let required = |name: &str| non_empty(name).ok_or_else(|| ConfigError(format!("{name} is required")));
        let server = required("AX_RUNNER_SERVER")?.trim_end_matches('/').to_owned();
        if !(server.starts_with("http://") || server.starts_with("https://")) {
            return Err(ConfigError(format!("AX_RUNNER_SERVER must be an http(s) URL, got {server:?}")));
        }
        let anthropic = TaskKeys {
            default: non_empty("AX_ANTHROPIC_API_KEY").map(Secret),
            precheck: non_empty("AX_ANTHROPIC_API_KEY_PRECHECK").map(Secret),
            report: non_empty("AX_ANTHROPIC_API_KEY_REPORT").map(Secret),
            work: non_empty("AX_ANTHROPIC_API_KEY_WORK").map(Secret),
        };
        if anthropic.for_task("report").is_none() || anthropic.for_task("work").is_none() {
            return Err(ConfigError(
                "the runner needs a Claude key for report and work (AX_ANTHROPIC_API_KEY, or _REPORT and _WORK)"
                    .to_owned(),
            ));
        }
        let timeout = match non_empty("AX_RUNNER_TIMEOUT") {
            None => Duration::from_secs(30 * MINUTE),
            Some(value) => parse_duration(&value).ok_or_else(|| {
                ConfigError(format!("AX_RUNNER_TIMEOUT must be a duration such as 30m, got {value:?}"))
            })?,
        };
        Ok(Self {
            server,
            token: Secret(required("AX_RUNNER_TOKEN")?),
            github_token: Secret(required("AX_GITHUB_TOKEN")?),
            github_api: non_empty("AX_GITHUB_API")
                .unwrap_or_else(|| "https://api.github.com".to_owned())
                .trim_end_matches('/')
                .to_owned(),
            anthropic,
            workdir: non_empty("AX_RUNNER_WORKDIR")
                .map_or_else(|| std::env::temp_dir().join("ax-runner"), PathBuf::from),
            timeout,
            claude: non_empty("AX_CLAUDE_BIN").unwrap_or_else(|| "claude".to_owned()),
            git_config: Vec::new(),
            poll: Duration::from_secs(5),
            heartbeat: Duration::from_secs(60),
        })
    }
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
        let login = LoginPolicy {
            attempts: match non_empty("AX_LOGIN_ATTEMPTS") {
                None => LoginPolicy::default().attempts,
                Some(value) => value.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    ConfigError(format!("AX_LOGIN_ATTEMPTS must be a whole number above 0, got {value:?}"))
                })?,
            },
            lockout: duration("AX_LOGIN_LOCKOUT", LoginPolicy::default().lockout)?,
        };

        let time_zone = non_empty("AX_TIME_ZONE").map_or_else(BusinessTimeZone::default, BusinessTimeZone);
        let provider = non_empty("AX_AI_PROVIDER").unwrap_or_else(|| "stub".to_owned());
        if provider != "stub" && provider != "anthropic" {
            return Err(ConfigError(format!("AX_AI_PROVIDER must be stub or anthropic, got {provider:?}")));
        }
        let scope_check = match non_empty("AX_SCOPE_CHECK").as_deref() {
            None | Some("true") => true,
            Some("false") => false,
            Some(other) => return Err(ConfigError(format!("AX_SCOPE_CHECK must be true or false, got {other:?}"))),
        };
        let secret = |name: &str| non_empty(name).map(Secret);
        let anthropic = TaskKeys {
            default: secret("AX_ANTHROPIC_API_KEY"),
            precheck: secret("AX_ANTHROPIC_API_KEY_PRECHECK"),
            report: secret("AX_ANTHROPIC_API_KEY_REPORT"),
            work: secret("AX_ANTHROPIC_API_KEY_WORK"),
        };
        if provider == "anthropic" && !anthropic.connected() {
            return Err(ConfigError(
                "AX_AI_PROVIDER=anthropic needs AX_ANTHROPIC_API_KEY (or a key per task)".to_owned(),
            ));
        }
        let runner_token = secret("AX_RUNNER_TOKEN");
        if runner_token.as_ref().is_some_and(|t| t.expose().len() < 32) {
            return Err(ConfigError("AX_RUNNER_TOKEN must be at least 32 characters".to_owned()));
        }
        let ai = AiSettings { provider, scope_check, anthropic, runner_token };

        let drafts = match non_empty("AX_DRAFTS_PER_ACCOUNT") {
            None => DraftPolicy::default(),
            Some(value) => DraftPolicy {
                per_account: value.parse().ok().filter(|n| *n > 0).ok_or_else(|| {
                    ConfigError(format!("AX_DRAFTS_PER_ACCOUNT must be a whole number above 0, got {value:?}"))
                })?,
            },
        };

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
            login,
            time_zone,
            ai,
            drafts,
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
        assert_eq!(c.ai, AiSettings::default());
        assert!(!c.ai.anthropic.connected());
        assert!(!config(&[("AX_SCOPE_CHECK", "false")]).unwrap().ai.scope_check);
        assert!(config(&[("AX_SCOPE_CHECK", "no")]).is_err());
        assert!(config(&[("AX_AI_PROVIDER", "magic")]).is_err());
        assert!(config(&[("AX_AI_PROVIDER", "anthropic")]).is_err(), "needs a key");
        let claude = config(&[("AX_AI_PROVIDER", "anthropic"), ("AX_ANTHROPIC_API_KEY_REPORT", "k-1")]).unwrap();
        assert_eq!(claude.ai.provider, "anthropic");
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
    fn provider_keys_per_task() {
        let c =
            config(&[("AX_ANTHROPIC_API_KEY", "sk-default-1111"), ("AX_ANTHROPIC_API_KEY_REPORT", " sk-report-2222 ")])
                .unwrap();
        let keys = &c.ai.anthropic;
        assert!(keys.connected());
        assert_eq!(keys.for_task("report").map(Secret::expose), Some("sk-report-2222"));
        assert_eq!(keys.for_task("work").map(Secret::expose), Some("sk-default-1111"));
        assert_eq!(keys.hint().as_deref(), Some("1111, 2222"));
        assert!(!format!("{c:?}").contains("sk-"), "keys never reach the log");

        let only_work = config(&[("AX_ANTHROPIC_API_KEY_WORK", "sk-work-3333")]).unwrap().ai.anthropic;
        assert_eq!(only_work.for_task("report"), None);
        assert_eq!(only_work.hint().as_deref(), Some("3333"));
        let same = config(&[("AX_ANTHROPIC_API_KEY", "k-9999"), ("AX_ANTHROPIC_API_KEY_WORK", "k-9999")]).unwrap();
        assert_eq!(same.ai.anthropic.hint().as_deref(), Some("9999"));
    }

    #[test]
    fn sign_in_lockout() {
        assert_eq!(config(&[]).unwrap().login, LoginPolicy { attempts: 5, lockout: Duration::from_secs(900) });
        let c = config(&[("AX_LOGIN_ATTEMPTS", "10"), ("AX_LOGIN_LOCKOUT", "1h")]).unwrap();
        assert_eq!(c.login, LoginPolicy { attempts: 10, lockout: Duration::from_secs(3600) });
        for (name, bad) in [("AX_LOGIN_ATTEMPTS", "0"), ("AX_LOGIN_ATTEMPTS", "lots"), ("AX_LOGIN_LOCKOUT", "soon")] {
            assert!(config(&[(name, bad)]).is_err(), "{name}={bad}");
        }
    }

    #[test]
    fn a_runner_token_is_long() {
        assert!(config(&[("AX_RUNNER_TOKEN", "short")]).is_err());
        let token = "r".repeat(32);
        assert_eq!(config(&[("AX_RUNNER_TOKEN", &token)]).unwrap().ai.runner_token, Some(Secret::new(token.clone())));
        assert!(!format!("{:?}", config(&[("AX_RUNNER_TOKEN", &token)]).unwrap()).contains(&token));
    }

    #[test]
    fn runner_settings() {
        let vars = |extra: &[(&'static str, &'static str)]| {
            let mut all: HashMap<String, String> = [
                ("AX_RUNNER_SERVER", "http://ax-backend:8080/"),
                ("AX_RUNNER_TOKEN", "t"),
                ("AX_GITHUB_TOKEN", "ghp_x"),
                ("AX_ANTHROPIC_API_KEY", "k"),
            ]
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
            for (k, v) in extra {
                all.insert((*k).to_owned(), (*v).to_owned());
            }
            RunnerConfig::from_lookup(move |name| all.get(name).cloned())
        };
        let c = vars(&[]).unwrap();
        assert_eq!((c.server.as_str(), c.github_api.as_str()), ("http://ax-backend:8080", "https://api.github.com"));
        assert_eq!((c.timeout, c.claude.as_str()), (Duration::from_secs(1800), "claude"));
        assert!(!format!("{c:?}").contains("ghp_x"));
        assert!(vars(&[("AX_RUNNER_SERVER", "ax-backend")]).is_err());
        assert!(vars(&[("AX_ANTHROPIC_API_KEY", "")]).is_err(), "no Claude key");
        assert!(vars(&[("AX_GITHUB_TOKEN", " ")]).is_err());
        assert!(vars(&[("AX_RUNNER_TIMEOUT", "soon")]).is_err());
    }

    #[test]
    fn drafts_per_account() {
        assert_eq!(config(&[]).unwrap().drafts.per_account, 20);
        assert_eq!(config(&[("AX_DRAFTS_PER_ACCOUNT", "5")]).unwrap().drafts.per_account, 5);
        for bad in ["0", "-1", "many", "2.5"] {
            assert!(config(&[("AX_DRAFTS_PER_ACCOUNT", bad)]).is_err(), "{bad}");
        }
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
