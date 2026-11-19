//! The AI's two jobs: the feasibility report and the work run (code change,
//! tests, merge request). A [`Provider`] does them; [`jobs`] queues them,
//! runs them outside any transaction, and applies their result as the AI's
//! lifecycle event (`reportReady`/`scopeReady`, `workFinished`).
//!
//! The AI only ever sends those events. It never sends a person's event, so
//! it cannot approve a merge, skip a review or move a request past a human
//! checkpoint.

pub mod handlers;
pub mod jobs;
pub mod settings;
pub mod usage;

use std::future::Future;

use serde_json::{Value, json};

use crate::lang::Lang;
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

/// Writes reports and runs work. Errors are retried by the job queue.
pub trait Provider: Send + Sync + 'static {
    fn report(&self, request: &StoredRequest, lang: Lang) -> impl Future<Output = Result<ReportResult, String>> + Send;
    fn work(&self, request: &StoredRequest, lang: Lang) -> impl Future<Output = Result<WorkResult, String>> + Send;
}

/// The provider until a real one is wired: it says so, in the request's
/// language, assesses nothing and changes no code. The lifecycle stays usable
/// end to end; the developer decides by hand.
#[derive(Debug, Clone, Copy, Default)]
pub struct StubProvider;

impl Provider for StubProvider {
    async fn report(&self, request: &StoredRequest, lang: Lang) -> Result<ReportResult, String> {
        let method = lang.pick("사람이 직접 확인", "Checked by hand");
        let verification: Vec<Value> = field(&request.details, "acceptanceCriteria")
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| json!({ "criterionId": c["id"], "automatable": false, "method": method }))
            .collect();
        let summary = lang.pick(
            "AI 제공자가 연결되지 않아 구현 가능성을 판단하지 않았습니다. 담당 개발자가 직접 검토해 주세요.",
            "No AI provider is connected, so feasibility was not assessed. The responsible developer reviews the request by hand.",
        );
        Ok(ReportResult {
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
        })
    }

    async fn work(&self, _request: &StoredRequest, lang: Lang) -> Result<WorkResult, String> {
        Ok(WorkResult {
            outcome: "error".to_owned(),
            summary: lang
                .pick(
                    "AI 제공자가 연결되지 않아 코드를 바꾸지 않았습니다. 직접 작업하거나, 제공자를 연결한 뒤 다시 실행해 주세요.",
                    "No AI provider is connected, so no code was changed. Do the work by hand, or connect a provider and run it again.",
                )
                .to_owned(),
            extra: serde_json::Map::new(),
            regression: Vec::new(),
        })
    }
}
