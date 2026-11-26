//! The server's side of the runner: three routes outside the contract, under
//! the API's base path, for `ax-api runner` (see `crate::runner`). With
//! `AX_RUNNER_TOKEN` set the server runs no AI jobs itself; a runner claims
//! them, renews its lease while it works, and posts the result, which is
//! applied exactly as the in-process worker applies its own.
//!
//! - `POST /runner/claim`: the next due job with everything the agent needs
//!   (`204` when there is none).
//! - `POST /runner/jobs/{id}/heartbeat`: renews the lease; a work run's phase.
//! - `POST /runner/jobs/{id}/result`: the report or work answer, or why there
//!   is none, and what the run cost.
//!
//! Every call carries `Authorization: Bearer <AX_RUNNER_TOKEN>`; calls about a
//! job also carry the claim the job was handed out with, so a runner that lost
//! its lease cannot write over the one that took the job next.

use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::jobs::{self, Done, Job, MAX_ATTEMPTS, Task};
use super::settings;
use super::usage::{self, Tokens};
use super::{Failure, Spend, agent};
use crate::error::{ApiError, ErrorCode};
use crate::lang::Lang;
use crate::requests::store::StoredRequest;
use crate::state::AppState;

/// How long a claim lasts without a heartbeat.
pub const LEASE: Duration = Duration::from_secs(10 * 60);

/// The work phases a runner may report while it works.
const PHASES: [&str; 3] = ["editing", "testing", "opening_mr"];

/// `404` when no runner is configured, `401` for a wrong token. Compared as
/// digests, so the comparison takes the same time whatever the token.
fn authorize(state: &AppState, headers: &HeaderMap, lang: Lang) -> Result<(), ApiError> {
    let Some(expected) = state.ai.runner_token.as_ref() else { return Err(ApiError::not_found(lang)) };
    let given = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if Sha256::digest(given.as_bytes()) == Sha256::digest(expected.expose().as_bytes()) {
        Ok(())
    } else {
        Err(ApiError::new(ErrorCode::Unauthorized, lang.pick("러너 토큰이 맞지 않습니다.", "Wrong runner token.")))
    }
}

/// `409` for a job that is not running under this claim (done, given up, or
/// claimed again after the lease ran out).
fn lost_claim(lang: Lang) -> ApiError {
    ApiError::new(
        ErrorCode::Conflict,
        lang.pick("이 작업은 더 이상 이 러너의 것이 아닙니다.", "This job is no longer this runner's."),
    )
}

/// The job, if it is running under `claim`.
async fn claimed(state: &AppState, id: i64, claim: Option<Uuid>, lang: Lang) -> Result<Job, ApiError> {
    let db = state.db(lang)?;
    let row = sqlx::query(
        "select id, request_id, task, lang, attempts, claim from ai_jobs
          where id = $1 and status = 'running' and claim is not null and claim = $2",
    )
    .bind(id)
    .bind(claim)
    .fetch_optional(db)
    .await
    .map_err(ApiError::db(lang))?
    .ok_or_else(|| lost_claim(lang))?;
    let task: String = row.try_get("task").map_err(ApiError::db(lang))?;
    let job_lang: String = row.try_get("lang").map_err(ApiError::db(lang))?;
    Ok(Job {
        id: row.try_get("id").map_err(ApiError::db(lang))?,
        request: row.try_get("request_id").map_err(ApiError::db(lang))?,
        task: if task == "work" { Task::Work } else { Task::Report },
        lang: if job_lang == "en" { Lang::En } else { Lang::Ko },
        attempts: row.try_get("attempts").map_err(ApiError::db(lang))?,
        claim: row.try_get("claim").map_err(ApiError::db(lang))?,
    })
}

fn claim_of(body: &Value) -> Option<Uuid> {
    body["claim"].as_str().and_then(|c| Uuid::parse_str(c).ok())
}

/// `POST /runner/claim`.
pub async fn claim(State(state): State<AppState>, lang: Lang, headers: HeaderMap) -> Result<Response, ApiError> {
    authorize(&state, &headers, lang)?;
    let db = state.db(lang)?;
    loop {
        let Some(job) = jobs::claim(db, Some(LEASE)).await.map_err(ApiError::db(lang))? else {
            return Ok(StatusCode::NO_CONTENT.into_response());
        };
        let Some(request) = jobs::request_for(db, &job).await.map_err(ApiError::db(lang))? else { continue };
        let context = jobs::context(db, &job, &request).await.map_err(ApiError::db(lang))?;
        if job.attempts > MAX_ATTEMPTS {
            // Claimed again and again without an answer: the runners keep failing on it.
            let failure = Failure { message: "the runner stopped answering".to_owned(), retry: false };
            jobs::complete(db, &job, &request, &context.selection, Err(failure), &[], &state.ai)
                .await
                .map_err(ApiError::db(lang))?;
            continue;
        }
        let mut conn = db.acquire().await.map_err(ApiError::db(lang))?;
        let limits = settings::budget(&mut conn).await.map_err(ApiError::db(lang))?;
        let pricing = settings::pricing(&mut conn).await.map_err(ApiError::db(lang))?;
        let spent = usage::spent_krw(
            &usage::for_request(&mut conn, &state.time_zone.0, request.id).await.map_err(ApiError::db(lang))?,
        );
        // What the request may still spend, for the agent's own cost cap.
        #[allow(clippy::cast_precision_loss)] // whole won up to 10^10 are exact in f64
        let budget_usd = (limits.per_request_limit_krw > 0)
            .then(|| ((limits.per_request_limit_krw as f64 - spent) / pricing.usd_krw).max(0.0));
        return Ok(Json(json!({
            "job": {
                "id": job.id,
                "claim": job.claim,
                "task": job.task.as_str(),
                "lang": job.lang.pick("ko", "en"),
                "attempt": job.attempts,
                "leaseSeconds": LEASE.as_secs(),
            },
            "selection": context.selection,
            "budgetUsd": budget_usd,
            "request": request.to_json(),
            "project": context.project,
        }))
        .into_response());
    }
}

/// `POST /runner/jobs/{id}/heartbeat` `{ claim, phase? }`: renews the lease
/// and, for a work run still in progress, shows how far it got.
pub async fn heartbeat(
    State(state): State<AppState>,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    authorize(&state, &headers, lang)?;
    let job = claimed(&state, id, claim_of(&body), lang).await?;
    let db = state.db(lang)?;
    sqlx::query("update ai_jobs set lease_until = now() + make_interval(secs => $2) where id = $1")
        .bind(job.id)
        .bind(LEASE.as_secs_f64())
        .execute(db)
        .await
        .map_err(ApiError::db(lang))?;
    if let (Task::Work, Some(phase)) = (job.task, body["phase"].as_str().filter(|p| PHASES.contains(p))) {
        // Only while the run is open: once its result is in, the request has moved on.
        sqlx::query(
            "update requests set details = jsonb_set(details, '{work,phase}', to_jsonb($2::text))
              where id = $1 and status = 'in_progress' and details ? 'work'",
        )
        .bind(job.request)
        .bind(phase)
        .execute(db)
        .await
        .map_err(ApiError::db(lang))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The calls a run made: `[{ model, tokens: { input, cacheRead, cacheWrite,
/// output }, costUsd? }]`, anything malformed left out. A runner runs the
/// provider's coding agent for every task, a report included, so each is
/// booked as that agent's run.
fn spend_of(body: &Value, provider: &str) -> Vec<Spend> {
    let runtime = settings::agent_runtime(provider);
    body["spend"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let model = s["model"].as_str().filter(|m| !m.is_empty())?.to_owned();
            let count = |key: &str| s["tokens"][key].as_i64().filter(|n| *n >= 0).unwrap_or(0);
            Some(Spend {
                model,
                runtime,
                tokens: Tokens {
                    input: count("input"),
                    cache_read: count("cacheRead"),
                    cache_write: count("cacheWrite"),
                    output: count("output"),
                    reasoning: None,
                },
                cost_usd: s["costUsd"].as_f64().filter(|c| *c >= 0.0),
            })
        })
        .collect()
}

/// `POST /runner/jobs/{id}/result` `{ claim, report | work | failure, spend }`:
/// ends the job as the in-process worker would.
pub async fn result(
    State(state): State<AppState>,
    lang: Lang,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    authorize(&state, &headers, lang)?;
    let job = claimed(&state, id, claim_of(&body), lang).await?;
    let db = state.db(lang)?;
    let mut conn = db.acquire().await.map_err(ApiError::db(lang))?;
    let request = StoredRequest::find(&mut conn, job.request)
        .await
        .map_err(ApiError::db(lang))?
        .ok_or_else(|| ApiError::request_not_found(lang, job.request))?;
    drop(conn);
    let context = jobs::context(db, &job, &request).await.map_err(ApiError::db(lang))?;
    let spend = spend_of(&body, &context.selection.provider);
    let model = spend.first().map_or(context.selection.model.as_str(), |s| s.model.as_str()).to_owned();
    let outcome = match (job.task, body.get("report"), body.get("work"), body.get("failure")) {
        (_, _, _, Some(failure)) => Err(Failure {
            message: failure["message"].as_str().unwrap_or("the runner failed").to_owned(),
            retry: failure["retry"].as_bool().unwrap_or(true),
        }),
        (Task::Report, Some(answer), _, _) => {
            Ok(Done::Report(agent::report(answer, &request, context.project.as_ref(), job.lang, &model)))
        }
        (Task::Work, _, Some(answer), _) => {
            Ok(Done::Work(agent::work(answer, &request, context.project.as_ref(), job.lang)))
        }
        _ => Err(Failure { message: "the runner's answer had no result for this task".to_owned(), retry: true }),
    };
    jobs::complete(db, &job, &request, &context.selection, outcome, &spend, &state.ai)
        .await
        .map_err(ApiError::db(lang))?;
    Ok(StatusCode::NO_CONTENT)
}
