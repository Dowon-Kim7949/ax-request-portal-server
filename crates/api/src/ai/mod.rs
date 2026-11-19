//! The AI's two jobs: the feasibility report and the work run (code change,
//! tests, merge request). A [`Provider`] does them; [`jobs`] queues them,
//! runs them outside any transaction, and applies their result as the AI's
//! lifecycle event (`reportReady`/`scopeReady`, `workFinished`).
//!
//! The AI only ever sends those events. It never sends a person's event, so
//! it cannot approve a merge, skip a review or move a request past a human
//! checkpoint.

pub mod agent;
pub mod claude;
pub mod handlers;
pub mod jobs;
pub mod runner;
pub mod settings;
pub mod usage;

use std::future::Future;

use serde_json::{Value, json};

use self::settings::Selection;
use self::usage::Tokens;
use crate::lang::Lang;
use crate::projects::Project;
use crate::requests::store::{StoredRequest, field};

/// What a provider writes for a report. `revision` and `generatedAt` are
/// filled in by the job.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportResult {
    /// The contract's `FeasibilityReport`, without `revision` and `generatedAt`.
    pub report: Value,
    /// The contract's `ScopeProposal` without `revision`, for a feasible or
    /// conditional report the requester confirms (`scope_check`).
    pub scope: Option<Value>,
}

/// What a provider reports after a work run; merged into the run.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkResult {
    /// `success`, `tests_failed` or `error`
    pub outcome: String,
    pub summary: String,
    /// `mergeRequest`, `tests`, `wikiChanges`, `preview`, as the contract's `WorkRun`.
    pub extra: serde_json::Map<String, Value>,
    /// Regression checks to run before release; empty keeps the current list.
    pub regression: Vec<Value>,
}

/// What a job is run for: the request's project, the model the settings
/// chose, and the language to write in.
#[derive(Debug, Clone)]
pub struct Job {
    pub lang: Lang,
    pub selection: Selection,
    /// `None` only if the project vanished, which the schema prevents.
    pub project: Option<Project>,
}

/// One billed model call or agent run: the model that ran it and its tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct Spend {
    pub model: String,
    pub tokens: Tokens,
    /// What the provider says the call cost, when it knows better than the
    /// price table (a coding agent's own estimate); booked as given.
    pub cost_usd: Option<f64>,
}

/// Why a job did not finish. `retry` is false when trying again would end
/// the same way (a refusal, a missing key, a rejected request).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    pub retry: bool,
}

/// A provider's answer: the result or why there is none, and what it cost
/// either way (the job books every spend, even for a failed or discarded run).
#[derive(Debug, Clone, PartialEq)]
pub struct Reply<T> {
    pub outcome: Result<T, Failure>,
    pub spend: Vec<Spend>,
}

impl<T> Reply<T> {
    pub fn ok(value: T) -> Self {
        Self { outcome: Ok(value), spend: Vec::new() }
    }

    /// A failure worth trying again later (an outage, a rate limit).
    pub fn retry(message: impl Into<String>) -> Self {
        Self { outcome: Err(Failure { message: message.into(), retry: true }), spend: Vec::new() }
    }

    /// A failure that would only repeat.
    pub fn fail(message: impl Into<String>) -> Self {
        Self { outcome: Err(Failure { message: message.into(), retry: false }), spend: Vec::new() }
    }

    #[must_use]
    pub fn with_spend(mut self, spend: Vec<Spend>) -> Self {
        self.spend = spend;
        self
    }
}

/// Writes reports and runs work.
pub trait Provider: Send + Sync + 'static {
    fn report(&self, request: &StoredRequest, job: &Job) -> impl Future<Output = Reply<ReportResult>> + Send;
    fn work(&self, request: &StoredRequest, job: &Job) -> impl Future<Output = Reply<WorkResult>> + Send;
}

/// A report that assesses nothing and leaves the request to the developer:
/// `undetermined`, every criterion to be checked by hand, `summary` saying
/// why. The stub writes it, and so does the queue when a report job gives up.
pub fn hand_over_report(request: &StoredRequest, lang: Lang, summary: &str) -> ReportResult {
    let method = lang.pick("사람이 직접 확인", "Checked by hand");
    let verification: Vec<Value> = field(&request.details, "acceptanceCriteria")
        .as_array()
        .into_iter()
        .flatten()
        .map(|c| json!({ "criterionId": c["id"], "automatable": false, "method": method }))
        .collect();
    ReportResult {
        report: json!({
            "verdict": "undetermined",
            "summary": summary,
            "evidence": { "wikiDocs": [], "codeFiles": [] },
            "impact": { "changedScreens": [], "recheck": [] },
            "plan": [],
            "verification": verification,
            "openQuestions": [],
            "assumptions": [],
            "constraintChecks": [],
            "wikiTrust": "low",
            "size": "M",
            "risks": [],
        }),
        scope: None,
    }
}

/// A finished work run that changed nothing (`outcome: error`), with `summary`.
pub fn hand_over_work(summary: &str) -> WorkResult {
    WorkResult {
        outcome: "error".to_owned(),
        summary: summary.to_owned(),
        extra: serde_json::Map::new(),
        regression: Vec::new(),
    }
}

/// The provider until a real one is wired: it says so, in the request's
/// language, assesses nothing and changes no code. The lifecycle stays usable
/// end to end; the developer decides by hand.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubProvider;

impl Provider for StubProvider {
    async fn report(&self, request: &StoredRequest, job: &Job) -> Reply<ReportResult> {
        let summary = job.lang.pick(
            "AI 제공자가 연결되지 않아 구현 가능성을 판단하지 않았습니다. 담당 개발자가 직접 검토해 주세요.",
            "No AI provider is connected, so feasibility was not assessed. The responsible developer reviews the request by hand.",
        );
        Reply::ok(hand_over_report(request, job.lang, summary))
    }

    async fn work(&self, _request: &StoredRequest, job: &Job) -> Reply<WorkResult> {
        Reply::ok(hand_over_work(job.lang.pick(
            "AI 제공자가 연결되지 않아 코드를 바꾸지 않았습니다. 직접 작업하거나, 제공자를 연결한 뒤 다시 실행해 주세요.",
            "No AI provider is connected, so no code was changed. Do the work by hand, or connect a provider and run it again.",
        )))
    }
}
