//! The runner (`ax-api runner`): takes the AI's jobs from a server and runs
//! them with Claude Code on a fresh clone of the project's GitHub repository.
//!
//! - A **report** clones the default branch and lets the agent read the code
//!   and the wiki. The runner keeps only the wiki pages and files that exist,
//!   leaves out the wiki's own navigation pages (index, README, profiles,
//!   log, templates, guides: they say nothing about the change), and reads
//!   each page's review state from its front matter; the server decides what
//!   the report may claim from that (`ai::agent`).
//! - A **work run** checks out the run's branch (`ax/request-<id>`, continued
//!   by later runs), checks with a dry-run push that the token may push there
//!   before the agent spends anything, lets the agent edit and test, commits
//!   what it changed, pushes the branch and opens (or reuses) the pull
//!   request. The agent holds no git credentials; the runner pushes. Nothing
//!   merges: a person does, and the token should not even allow it. A wiki
//!   page the agent edited is never left marked `verified`: it goes back to
//!   `needs_review`.
//! - A token that is refused (signed out, no right to push, no access to the
//!   repository) fails the job for good: trying again would only run the
//!   agent again for nothing (`git::refused`).
//!
//! The runner has no database access. It talks to the server over HTTP
//! (`ai::runner`), renews its claim while it works, and reports what each run
//! cost. One job at a time; run more runners for more.

pub mod agent;
pub mod git;
pub mod github;

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use self::agent::{Mode, Run};
use self::git::{Git, Repo};
use self::github::GitHub;
use crate::config::RunnerConfig;

/// Why a job has no result. `spend` is what it cost all the same.
#[derive(Debug, Clone, PartialEq)]
pub struct RunError {
    pub message: String,
    pub retry: bool,
    pub spend: Vec<Value>,
}

impl RunError {
    pub fn retry(message: impl Into<String>) -> Self {
        Self { message: message.into(), retry: true, spend: Vec::new() }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self { message: message.into(), retry: false, spend: Vec::new() }
    }

    #[must_use]
    pub fn with_spend(mut self, spend: Vec<Value>) -> Self {
        if self.spend.is_empty() {
            self.spend = spend;
        }
        self
    }
}

/// A claimed job, as `POST /runner/claim` hands it out.
struct Claimed {
    id: i64,
    claim: String,
    task: String,
    lang: String,
    model: String,
    budget_usd: Option<f64>,
    request: Value,
    project: Value,
}

impl Claimed {
    fn parse(body: &Value) -> Option<Self> {
        Some(Self {
            id: body["job"]["id"].as_i64()?,
            claim: body["job"]["claim"].as_str()?.to_owned(),
            task: body["job"]["task"].as_str()?.to_owned(),
            lang: body["job"]["lang"].as_str().unwrap_or("ko").to_owned(),
            model: body["selection"]["model"].as_str()?.to_owned(),
            budget_usd: body["budgetUsd"].as_f64(),
            request: body["request"].clone(),
            project: body["project"].clone(),
        })
    }

    fn wiki_path(&self) -> String {
        self.project["wikiPath"].as_str().unwrap_or("docs/llm-wiki").trim_matches('/').to_owned()
    }
}

pub struct Runner {
    config: RunnerConfig,
    http: reqwest::Client,
}

impl Runner {
    /// # Panics
    ///
    /// If the HTTP client cannot be built (no TLS backend), a build defect.
    pub fn new(config: RunnerConfig) -> Self {
        let http =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().expect("an HTTP client");
        Self { config, http }
    }

    /// Takes jobs until the process is stopped.
    pub async fn run(&self) {
        tracing::info!(server = %self.config.server, "runner started");
        loop {
            match self.run_once().await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(self.config.poll).await,
                Err(error) => {
                    tracing::warn!(%error, "runner");
                    tokio::time::sleep(self.config.poll).await;
                }
            }
        }
    }

    /// Takes and runs one job, if there is one. Returns whether there was.
    ///
    /// # Errors
    ///
    /// When the server cannot be reached or refuses the runner.
    pub async fn run_once(&self) -> Result<bool, String> {
        let response = self
            .http
            .post(format!("{}/runner/claim", self.config.server))
            .bearer_auth(self.config.token.expose())
            .send()
            .await
            .map_err(|e| format!("server unreachable: {e}"))?;
        match response.status().as_u16() {
            204 => return Ok(false),
            200 => {}
            status => return Err(format!("the server refused the claim ({status})")),
        }
        let body: Value = response.json().await.map_err(|e| format!("bad claim: {e}"))?;
        let job = Claimed::parse(&body).ok_or("the claim had no job")?;
        self.handle(&job).await;
        Ok(true)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<u16, String> {
        let response = self
            .http
            .post(format!("{}{path}", self.config.server))
            .bearer_auth(self.config.token.expose())
            .json(body)
            .send()
            .await
            .map_err(|e| format!("server unreachable: {e}"))?;
        Ok(response.status().as_u16())
    }

    async fn handle(&self, job: &Claimed) {
        tracing::info!(job = job.id, task = %job.task, "job claimed");
        let phase: Arc<Mutex<Option<String>>> = Arc::default();
        let beat = {
            let (http, server, token) = (self.http.clone(), self.config.server.clone(), self.config.token.clone());
            let (every, id, claim, phase) = (self.config.heartbeat, job.id, job.claim.clone(), Arc::clone(&phase));
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(every).await;
                    let current = phase.lock().map(|p| p.clone()).unwrap_or_default();
                    let sent = http
                        .post(format!("{server}/runner/jobs/{id}/heartbeat"))
                        .bearer_auth(token.expose())
                        .json(&json!({ "claim": claim, "phase": current }))
                        .send()
                        .await;
                    if sent.is_ok_and(|r| r.status().as_u16() == 409) {
                        tracing::warn!(job = id, "the claim was lost");
                        break;
                    }
                }
            })
        };
        let short: String = job.claim.chars().take(8).collect();
        let dir = self.config.workdir.join(format!("job-{}-{short}", job.id));
        let outcome = match job.task.as_str() {
            "report" => self.report(job, &dir).await.map(|(a, s)| ("report", a, s)),
            "work" => self.work(job, &dir, &phase).await.map(|(a, s)| ("work", a, s)),
            other => Err(RunError::fail(format!("unknown task {other}"))),
        };
        beat.abort();
        let _ = tokio::fs::remove_dir_all(&dir).await;
        let body = match outcome {
            Ok((key, answer, spend)) => json!({ "claim": job.claim, key: answer, "spend": spend }),
            Err(e) => {
                tracing::warn!(job = job.id, error = %e.message, retry = e.retry, "job failed");
                json!({ "claim": job.claim, "failure": { "message": e.message, "retry": e.retry }, "spend": e.spend })
            }
        };
        match self.post(&format!("/runner/jobs/{}/result", job.id), &body).await {
            Ok(204) => tracing::info!(job = job.id, "result delivered"),
            Ok(status) => tracing::warn!(job = job.id, status, "the server did not take the result"),
            Err(error) => tracing::warn!(job = job.id, %error, "the result was not delivered"),
        }
    }

    async fn set_phase(&self, job: &Claimed, phase: &Mutex<Option<String>>, value: &str) {
        if let Ok(mut p) = phase.lock() {
            *p = Some(value.to_owned());
        }
        let body = json!({ "claim": job.claim, "phase": value });
        if let Err(error) = self.post(&format!("/runner/jobs/{}/heartbeat", job.id), &body).await {
            tracing::warn!(job = job.id, %error, "phase not reported");
        }
    }

    /// The project's repository, cloned into `dir/repo` at the default branch.
    async fn clone(&self, job: &Claimed, dir: &Path) -> Result<(Repo, PathBuf, String), RunError> {
        let repository = &job.project["repository"];
        if repository["provider"] != "github" {
            return Err(RunError::fail("only GitHub repositories are wired to the runner"));
        }
        let url = repository["url"].as_str().unwrap_or_default();
        let repo = Repo::parse(url).ok_or_else(|| RunError::fail(format!("not a GitHub repository URL: {url}")))?;
        tokio::fs::create_dir_all(dir).await.map_err(|e| RunError::retry(format!("no work directory: {e}")))?;
        let git = self.git(&repo);
        git.run(dir, &["clone", "--depth", "50", "--no-tags", &repo.url, "repo"]).await?;
        let clone = dir.join("repo");
        let base = git.run(&clone, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        Ok((repo, clone, base))
    }

    fn git<'a>(&'a self, repo: &'a Repo) -> Git<'a> {
        Git { token: self.config.github_token.expose(), repo, config: &self.config.git_config }
    }

    fn agent<'a>(
        &'a self,
        job: &'a Claimed,
        dir: &'a Path,
        prompt: &'a str,
        schema: &'a Value,
        mode: Mode,
    ) -> Result<Run<'a>, RunError> {
        let key = self
            .config
            .anthropic
            .for_task(&job.task)
            .ok_or_else(|| RunError::fail(format!("no Claude key for the {} task", job.task)))?;
        Ok(Run {
            bin: &self.config.claude,
            dir,
            api_key: key.expose(),
            model: &job.model,
            prompt,
            schema,
            mode,
            budget_usd: job.budget_usd,
            timeout: self.config.timeout,
        })
    }

    async fn report(&self, job: &Claimed, dir: &Path) -> Result<(Value, Vec<Value>), RunError> {
        let (_, clone, _) = self.clone(job, dir).await?;
        let prompt = agent::report_prompt(&job.request, &job.project, &job.lang);
        let schema = agent::report_schema();
        let (mut answer, spend) = agent::run(&self.agent(job, &clone, &prompt, &schema, Mode::Read)?).await?;
        // Only evidence that exists and is about the software, with the review
        // state the wiki itself records.
        let wiki = job.wiki_path();
        let docs: Vec<Value> = answer["wikiDocs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|doc| {
                let path = relative(doc["path"].as_str()?)?;
                let inside = path.strip_prefix(&format!("{wiki}/"))?;
                let text = std::fs::read_to_string(clone.join(&path)).ok()?;
                let page = front_matter(&text);
                if navigation_page(inside, page.doc_type.as_deref()) {
                    return None;
                }
                let title =
                    page.title.or_else(|| doc["title"].as_str().map(str::to_owned)).unwrap_or_else(|| path.clone());
                Some(json!({ "path": path, "title": title, "verified": page.status.as_deref() == Some("verified") }))
            })
            .collect();
        let files: Vec<String> = answer["codeFiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| relative(f.as_str()?))
            .filter(|f| clone.join(f).is_file())
            .collect();
        answer["wikiDocs"] = json!(docs);
        answer["codeFiles"] = json!(files);
        Ok((answer, spend))
    }

    async fn work(
        &self,
        job: &Claimed,
        dir: &Path,
        phase: &Mutex<Option<String>>,
    ) -> Result<(Value, Vec<Value>), RunError> {
        let (repo, clone, base) = self.clone(job, dir).await?;
        let git = self.git(&repo);
        let id = job.request["id"].as_i64().unwrap_or(job.id);
        let branch = job.request["work"]["branch"].as_str().map_or_else(|| format!("ax/request-{id}"), str::to_owned);
        let remote = git.run(&clone, &["ls-remote", "--heads", "origin", &branch]).await?;
        if remote.is_empty() {
            git.run(&clone, &["checkout", "-b", &branch]).await?;
        } else {
            // A later run continues the branch, and the pull request, of the first.
            let refspec = format!("refs/heads/{branch}:refs/remotes/origin/{branch}");
            git.run(&clone, &["fetch", "--depth", "50", "origin", &refspec]).await?;
            git.run(&clone, &["checkout", "-b", &branch, &format!("origin/{branch}")]).await?;
        }
        // Before the agent spends anything: may the token push here? A dry run
        // asks the remote for the right to push and sends nothing, so a token
        // without it ends the run now, at no cost, instead of after the edit.
        git.run(&clone, &["push", "--dry-run", "origin", &format!("HEAD:refs/heads/{branch}")])
            .await
            .map_err(|e| RunError { message: format!("the token cannot push to {}: {}", repo.url, e.message), ..e })?;

        self.set_phase(job, phase, "editing").await;
        let prompt = agent::work_prompt(&job.request, &job.project, &job.lang);
        let schema = agent::work_schema();
        let (mut answer, spend) = agent::run(&self.agent(job, &clone, &prompt, &schema, Mode::Edit)?).await?;
        let after = |e: RunError| e.with_spend(spend.clone());

        if git.run(&clone, &["status", "--porcelain"]).await.map_err(after)?.is_empty() {
            answer["outcome"] = json!("error");
            let none = if job.lang == "en" {
                "The agent changed no files."
            } else {
                "에이전트가 바꾼 파일이 없습니다."
            };
            let summary = answer["summary"].as_str().unwrap_or_default();
            answer["summary"] = json!(format!("{none} {summary}").trim().to_owned());
            return Ok((answer, spend));
        }
        let wiki = job.wiki_path();
        unverify_wiki(&clone, &git, &wiki).await.map_err(after)?;

        self.set_phase(job, phase, "opening_mr").await;
        let title = job.request["title"].as_str().unwrap_or_default();
        let attempt = job.request["work"]["attempt"].as_i64().unwrap_or(1);
        git.run(&clone, &["add", "-A"]).await.map_err(after)?;
        let message = format!("[AX #{id}] {title} (run {attempt})");
        git.run(
            &clone,
            &[
                "-c",
                "user.name=AX Request Portal",
                "-c",
                "user.email=ax-runner@users.noreply.github.com",
                "commit",
                "-q",
                "-m",
                &message,
            ],
        )
        .await
        .map_err(after)?;
        git.run(&clone, &["push", "origin", &format!("HEAD:refs/heads/{branch}")]).await.map_err(after)?;
        let changed = git.run(&clone, &["diff", "--name-only", "HEAD^", "HEAD"]).await.map_err(after)?;
        let docs: Vec<&str> = changed.lines().filter(|f| f.starts_with(&format!("{wiki}/"))).collect();

        let github =
            GitHub { http: &self.http, api: &self.config.github_api, token: self.config.github_token.expose() };
        let summary = answer["summary"].as_str().unwrap_or_default();
        let body = format!(
            "Request #{id}: {title}\n\n{summary}\n\nOpened by the AX Request Portal's AI. A developer reviews and merges it; \
             wiki edits wait for their own review."
        );
        let pr =
            github.pull_request(&repo, &branch, &base, &format!("[AX #{id}] {title}"), &body).await.map_err(after)?;
        answer["mergeRequest"] = pr;
        answer["wikiDocs"] = json!(docs);
        Ok((answer, spend))
    }
}

/// A path inside the repository, as given relative to its root; `None` for
/// anything that would leave it.
fn relative(path: &str) -> Option<String> {
    let path = path.trim().trim_start_matches("./");
    let p = Path::new(path);
    let inside = !path.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)));
    inside.then(|| path.to_owned())
}

/// What a Markdown page's front matter says about it.
#[derive(Debug, Default, PartialEq, Eq)]
struct FrontMatter {
    status: Option<String>,
    title: Option<String>,
    doc_type: Option<String>,
}

fn front_matter(text: &str) -> FrontMatter {
    let mut page = FrontMatter::default();
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return page };
    for line in rest.lines().take_while(|l| l.trim_end() != "---") {
        let value = |l: &str| {
            l.split_once(':').map(|(_, v)| v.trim().trim_matches(['"', '\'']).to_owned()).filter(|v| !v.is_empty())
        };
        if line.starts_with("status:") {
            page.status = value(line);
        } else if line.starts_with("title:") {
            page.title = value(line);
        } else if line.starts_with("doc_type:") {
            page.doc_type = value(line);
        }
    }
    page
}

/// The `doc_type`s of a wiki's own navigation and bookkeeping pages, as the
/// LLM-WIKI tools write them (they differ in spelling).
const NAVIGATION_TYPES: [&str; 12] = [
    "index",
    "wiki_index",
    "readme",
    "wiki_readme",
    "project_profile",
    "profile",
    "change_log",
    "changelog",
    "log",
    "template",
    "guide",
    "agent_guide",
];

/// Whether a wiki page (its path inside the wiki) is one of the wiki's own
/// navigation or bookkeeping pages rather than a page about the software:
/// the index, README, project and stack profiles, the change log, templates
/// and agent guides. The agent reads them to find its way, but they are no
/// evidence for a request, and their review state says nothing about the
/// pages that are. Known by `doc_type`, or by the standard layout when a
/// tool names the type differently.
fn navigation_page(inside: &str, doc_type: Option<&str>) -> bool {
    let by_type = doc_type
        .map(|t| t.trim().to_ascii_lowercase().replace('-', "_"))
        .is_some_and(|t| NAVIGATION_TYPES.contains(&t.as_str()));
    let by_path = matches!(inside, "index.md" | "README.md" | "project-profile.md" | "log.md")
        || ["templates/", "profiles/"].iter().any(|dir| inside.starts_with(dir));
    by_type || by_path
}

/// The AI never approves its own wiki edits: a page it changed that says
/// `status: verified` goes back to `needs_review`.
async fn unverify_wiki(clone: &Path, git: &Git<'_>, wiki: &str) -> Result<(), RunError> {
    // Changed tracked pages, then new ones.
    let mut paths = git.run(clone, &["diff", "--name-only", "HEAD", "--", wiki]).await?;
    paths.push('\n');
    paths.push_str(&git.run(clone, &["ls-files", "--others", "--exclude-standard", "--", wiki]).await?);
    for path in paths.lines().map(str::trim).filter(|p| !p.is_empty()) {
        let file = clone.join(path);
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        if front_matter(&text).status.as_deref() != Some("verified") {
            continue;
        }
        let mut in_front = false;
        let mut out = String::with_capacity(text.len());
        for (i, line) in text.split_inclusive('\n').enumerate() {
            if line.trim_end() == "---" {
                in_front = i == 0;
            }
            if in_front && line.starts_with("status:") {
                out.push_str(&line.replacen("verified", "needs_review", 1));
            } else {
                out.push_str(line);
            }
        }
        std::fs::write(&file, out).map_err(|e| RunError::retry(format!("could not write {path}: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_stay_inside_the_repository() {
        assert_eq!(relative("./docs/llm-wiki/a.md").as_deref(), Some("docs/llm-wiki/a.md"));
        for bad in ["../etc/passwd", "/etc/passwd", "docs/../../x", ""] {
            assert_eq!(relative(bad), None, "{bad}");
        }
    }

    #[test]
    fn front_matter_gives_status_title_and_type() {
        let page = "---\ntitle: \"Notices\"\nstatus: verified\ndoc_type: domain\n---\n# Notices\nstatus: not here\n";
        let expected = FrontMatter {
            status: Some("verified".into()),
            title: Some("Notices".into()),
            doc_type: Some("domain".into()),
        };
        assert_eq!(front_matter(page), expected);
        assert_eq!(front_matter("# No front matter"), FrontMatter::default());
    }

    #[test]
    fn navigation_pages_are_no_evidence() {
        for (inside, doc_type) in [
            ("index.md", None),
            ("README.md", Some("overview")),
            ("project-profile.md", None),
            ("log.md", None),
            ("templates/TASK_PROMPT.template.md", None),
            ("profiles/frontend.md", None),
            ("start.md", Some("wiki_index")),
            ("start.md", Some("Wiki-Index")),
            ("llm-wiki-guide/ai-instructions.md", Some("guide")),
            ("history.md", Some("changelog")),
        ] {
            assert!(navigation_page(inside, doc_type), "{inside} {doc_type:?}");
        }
        for (inside, doc_type) in [
            ("domains/04_requests.md", Some("domain")),
            ("DOMAIN_FEATURES.md", Some("domain-index")),
            ("domains/00_overview.md", Some("domain_overview")),
            ("notices.md", None),
            ("domains/index.md", Some("domain")),
        ] {
            assert!(!navigation_page(inside, doc_type), "{inside} {doc_type:?}");
        }
    }
}
