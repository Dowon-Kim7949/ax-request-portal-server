//! `listRepositoryBranches`: a repository's branches, for the project form.
//!
//! The server asks GitHub with its own read token (`AX_GITHUB_READ_TOKEN`),
//! never with the runner's, which can push. A repository it cannot read is
//! the mock's `repositoryNotFound` (`404`): no read token, a GitLab
//! repository (this server connects to GitHub only), a URL that names no
//! `owner/name`, or anything GitHub refuses or fails to answer. The form then
//! takes a typed name, so none of these blocks saving a project.

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{RawQuery, State};
use serde::Serialize;
use serde_json::Value;

use super::handlers::{PROVIDER, PROVIDERS, URL, is_web_url};
use crate::auth::{self, CurrentUser};
use crate::error::{ApiError, Issue};
use crate::lang::Lang;
use crate::roles::Permission;
use crate::runner::git::Repo;
use crate::state::AppState;

/// GitHub's largest page.
pub const PER_PAGE: usize = 100;
/// Pages read at most; a repository with more branches is `truncated`.
pub const MAX_PAGES: usize = 10;
/// One GitHub call; the form waits for the whole answer.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The contract's `RepositoryBranches`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryBranches {
    pub default_branch: String,
    /// The default branch first, then the rest by name.
    pub branches: Vec<String>,
    /// More branches than [`MAX_PAGES`] pages hold (or exactly that many).
    pub truncated: bool,
}

/// `GET /repositories/branches?provider=&url=` (`project:manage`). The
/// mock's order: session, then the query (`400`), then the permission
/// (`403`), then the repository (`404`).
pub async fn list(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    RawQuery(query): RawQuery,
) -> Result<Json<RepositoryBranches>, ApiError> {
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).into_owned().collect();
    let provider = params.get("provider").filter(|p| PROVIDERS.contains(&p.as_str()));
    let url = params.get("url").filter(|u| is_web_url(u));
    let mut issues = Vec::new();
    for (path, missing, (ko, en)) in [("provider", provider.is_none(), PROVIDER), ("url", url.is_none(), URL)] {
        if missing {
            issues.push(Issue { path: path.to_owned(), message: lang.pick(ko, en).to_owned() });
        }
    }
    let (Some(provider), Some(url)) = (provider, url) else {
        return Err(ApiError::bad_query(lang, issues));
    };
    auth::require(&current.user, Permission::ProjectManage, lang)?;

    let not_found = || ApiError::repository_not_found(lang);
    if provider != "github" {
        return Err(not_found());
    }
    let Some(token) = &state.github.token else {
        tracing::info!("no AX_GITHUB_READ_TOKEN: repository branches are not read");
        return Err(not_found());
    };
    let repo = Repo::parse(url).ok_or_else(not_found)?;
    match read(&state.http, &state.github.api, token.expose(), &repo).await {
        Ok(branches) => Ok(Json(branches)),
        Err(reason) => {
            tracing::warn!(owner = %repo.owner, name = %repo.name, %reason, "repository branches not read");
            Err(not_found())
        }
    }
}

/// The repository's default branch and up to [`MAX_PAGES`] pages of branches.
async fn read(http: &reqwest::Client, api: &str, token: &str, repo: &Repo) -> Result<RepositoryBranches, String> {
    let base = format!("{api}/repos/{}/{}", repo.owner, repo.name);
    let about = get(http, &base, token).await?;
    let default_branch = about["default_branch"].as_str().ok_or("GitHub named no default branch")?.to_owned();
    let mut names = Vec::new();
    let mut truncated = false;
    for page in 1..=MAX_PAGES {
        let list = get(http, &format!("{base}/branches?per_page={PER_PAGE}&page={page}"), token).await?;
        let list = list.as_array().ok_or("GitHub's branch list is not a list")?;
        names.extend(list.iter().filter_map(|b| b["name"].as_str().map(str::to_owned)));
        if list.len() < PER_PAGE {
            break;
        }
        truncated = page == MAX_PAGES;
    }
    Ok(ordered(default_branch, names, truncated))
}

async fn get(http: &reqwest::Client, url: &str, token: &str) -> Result<Value, String> {
    let response = http
        .get(url)
        .timeout(TIMEOUT)
        .bearer_auth(token)
        .header("accept", "application/vnd.github+json")
        .header("x-github-api-version", "2022-11-28")
        .header("user-agent", "ax-request-portal")
        .send()
        .await
        .map_err(|e| format!("GitHub unreachable: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("GitHub answered {status}"));
    }
    response.json().await.map_err(|e| format!("GitHub answered no JSON: {e}"))
}

/// The default branch first, then the rest by name, each once.
fn ordered(default_branch: String, mut names: Vec<String>, truncated: bool) -> RepositoryBranches {
    names.retain(|n| *n != default_branch);
    names.sort();
    names.dedup();
    names.insert(0, default_branch.clone());
    RepositoryBranches { default_branch, branches: names, truncated }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_branch_comes_first() {
        let names = ["release/2", "main", "develop", "develop"].map(str::to_owned).to_vec();
        assert_eq!(
            ordered("main".to_owned(), names, false),
            RepositoryBranches {
                default_branch: "main".to_owned(),
                branches: ["main", "develop", "release/2"].map(str::to_owned).to_vec(),
                truncated: false,
            }
        );
        // Listed even when the pages read did not reach it.
        assert_eq!(ordered("trunk".to_owned(), vec!["a".to_owned()], true).branches, ["trunk", "a"]);
    }
}
