//! Git on the job's clone. Credentials go in per command, as an
//! `http.<origin>.extraHeader` in the environment, so they are never written
//! to the clone's config or put where the agent could read them.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::process::Command;

use super::RunError;

/// A GitHub repository from its web URL: `https://host/owner/repo(.git)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    /// `https://host/`, where the token is sent.
    pub origin: String,
    pub url: String,
    pub owner: String,
    pub name: String,
}

impl Repo {
    pub fn parse(url: &str) -> Option<Self> {
        let parsed = url::Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "https" | "http") {
            return None;
        }
        let mut parts = parsed.path_segments()?.filter(|s| !s.is_empty());
        let owner = parts.next()?.to_owned();
        let name = parts.next()?.trim_end_matches(".git").to_owned();
        if parts.next().is_some() || name.is_empty() {
            return None;
        }
        let origin = format!("{}://{}/", parsed.scheme(), parsed.host_str()?);
        Some(Self { origin, url: url.trim_end_matches('/').to_owned(), owner, name })
    }
}

/// Runs a git command and returns its standard output.
pub struct Git<'a> {
    pub token: &'a str,
    pub repo: &'a Repo,
    pub config: &'a [(String, String)],
}

impl Git<'_> {
    pub async fn run(&self, dir: &Path, args: &[&str]) -> Result<String, RunError> {
        let mut config: Vec<(String, String)> = self.config.to_vec();
        let basic = STANDARD.encode(format!("x-access-token:{}", self.token));
        config.push((format!("http.{}.extraheader", self.repo.origin), format!("AUTHORIZATION: basic {basic}")));
        let mut command = Command::new("git");
        command
            .args(args)
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            // English messages, which `refused` reads.
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_COUNT", config.len().to_string());
        for (i, (key, value)) in config.iter().enumerate() {
            command.env(format!("GIT_CONFIG_KEY_{i}"), key).env(format!("GIT_CONFIG_VALUE_{i}"), value);
        }
        let out = command.output().await.map_err(|e| RunError::retry(format!("git could not start: {e}")))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        } else {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // Never repeat the token, should git print it.
            let stderr = stderr.replace(self.token, "***");
            let message = format!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim());
            Err(if refused(&stderr) { RunError::fail(message) } else { RunError::retry(message) })
        }
    }
}

/// Whether git failed because the token may not do this, which trying again
/// does not change: a failed sign-in, a token without the right to push, or a
/// repository it cannot see (GitHub answers a private one with 404).
fn refused(stderr: &str) -> bool {
    const SIGNS: [&str; 7] = [
        "The requested URL returned error: 401",
        "The requested URL returned error: 403",
        "The requested URL returned error: 404",
        "Authentication failed",
        "Write access to repository not granted",
        "Repository not found",
        "denied to",
    ];
    SIGNS.iter().any(|sign| stderr.contains(sign))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_from_web_urls() {
        let r = Repo::parse("https://github.com/acme/portal.git").unwrap();
        assert_eq!((r.origin.as_str(), r.owner.as_str(), r.name.as_str()), ("https://github.com/", "acme", "portal"));
        assert_eq!(Repo::parse("https://ghe.example.com/team/app/").unwrap().name, "app");
        for bad in ["git@github.com:acme/portal.git", "https://github.com/acme", "https://github.com/a/b/c", "nope"] {
            assert_eq!(Repo::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_refused_token_is_not_tried_again() {
        for refusal in [
            "remote: Write access to repository not granted.\nfatal: unable to access 'https://github.com/acme/portal/': The requested URL returned error: 403",
            "fatal: Authentication failed for 'https://github.com/acme/portal/'",
            "remote: Repository not found.\nfatal: repository 'https://github.com/acme/portal/' not found",
            "remote: Permission to acme/portal.git denied to someone.",
        ] {
            assert!(refused(refusal), "{refusal}");
        }
        for passing in [
            "fatal: unable to access 'https://github.com/acme/portal/': Could not resolve host: github.com",
            "fatal: unable to access 'https://github.com/acme/portal/': The requested URL returned error: 502",
            "error: failed to push some refs to 'https://github.com/acme/portal'",
        ] {
            assert!(!refused(passing), "{passing}");
        }
    }
}
