//! The GitHub calls a work run needs: its pull request, opened once and
//! reused by later runs on the same branch, or only looked up when a run
//! changed nothing. The token may push and open pull requests; nothing here
//! merges, and the token should not be able to.

use serde_json::{Value, json};

use super::RunError;
use super::git::Repo;

pub struct GitHub<'a> {
    pub http: &'a reqwest::Client,
    pub api: &'a str,
    pub token: &'a str,
}

impl GitHub<'_> {
    async fn call(&self, request: reqwest::RequestBuilder) -> Result<Value, RunError> {
        let response = request
            .bearer_auth(self.token)
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .header("user-agent", "ax-request-portal-runner")
            .send()
            .await
            .map_err(|e| RunError::retry(format!("GitHub unreachable: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            return Ok(body);
        }
        let message = format!("GitHub {status}: {}", body["message"].as_str().unwrap_or(""));
        Err(if status.as_u16() == 429 || status.is_server_error() {
            RunError::retry(message)
        } else {
            RunError::fail(message)
        })
    }

    /// The open pull request from `branch`, or a new one into `base`; with
    /// its size, as the contract's `MergeRequestInfo`.
    pub async fn pull_request(
        &self,
        repo: &Repo,
        branch: &str,
        base: &str,
        title: &str,
        body: &str,
    ) -> Result<Value, RunError> {
        let number = match self.open_number(repo, branch).await? {
            Some(number) => number,
            None => {
                let created = self
                    .call(
                        self.http
                            .post(pulls(self.api, repo))
                            .json(&json!({ "title": title, "head": branch, "base": base, "body": body })),
                    )
                    .await?;
                created["number"].as_u64().ok_or_else(|| RunError::retry("GitHub returned no pull request number"))?
            }
        };
        self.info(repo, number).await
    }

    /// The open pull request from `branch`, if there is one, as the
    /// contract's `MergeRequestInfo`. Opens none.
    pub async fn open_pull_request(&self, repo: &Repo, branch: &str) -> Result<Option<Value>, RunError> {
        match self.open_number(repo, branch).await? {
            Some(number) => self.info(repo, number).await.map(Some),
            None => Ok(None),
        }
    }

    async fn open_number(&self, repo: &Repo, branch: &str) -> Result<Option<u64>, RunError> {
        let head = format!("{}:{branch}", repo.owner);
        let open = self.call(self.http.get(pulls(self.api, repo)).query(&[("state", "open"), ("head", &head)])).await?;
        Ok(open.as_array().and_then(|list| list.first()).and_then(|pr| pr["number"].as_u64()))
    }

    async fn info(&self, repo: &Repo, number: u64) -> Result<Value, RunError> {
        let pr = self.call(self.http.get(format!("{}/{number}", pulls(self.api, repo)))).await?;
        Ok(json!({
            "number": number,
            "url": pr["html_url"],
            "filesChanged": pr["changed_files"].as_u64().unwrap_or(0),
            "additions": pr["additions"].as_u64().unwrap_or(0),
            "deletions": pr["deletions"].as_u64().unwrap_or(0),
        }))
    }
}

fn pulls(api: &str, repo: &Repo) -> String {
    format!("{api}/repos/{}/{}/pulls", repo.owner, repo.name)
}
