//! The AI's job queue (`ai_jobs`): queued when a request reaches `reporting`
//! or `in_progress`, claimed with `FOR UPDATE SKIP LOCKED`, run outside any
//! transaction, and applied only if the request is still where it was.

use std::time::Duration;

use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Row, Transaction};

use uuid::Uuid;

use super::settings::{self, Selection};
use super::usage::{self, Call};
use super::{Job as JobContext, Provider, Spend};
use crate::config::AiSettings;
use crate::lang::Lang;
use crate::notifications;
use crate::projects::Project;
use crate::requests::lifecycle::{Event, Status};
use crate::requests::store::{self, Actor, StoredRequest, field};

/// Tries before a job is given up; the request then waits for a person.
pub const MAX_ATTEMPTS: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    Report,
    Work,
}

impl Task {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Work => "work",
        }
    }

    /// The status a request waits in while the job runs.
    pub const fn waits_in(self) -> Status {
        match self {
            Self::Report => Status::Reporting,
            Self::Work => Status::InProgress,
        }
    }

    /// The job a request needs on reaching `status`, if any.
    pub const fn for_status(status: Status) -> Option<Self> {
        match status {
            Status::Reporting => Some(Self::Report),
            Status::InProgress => Some(Self::Work),
            _ => None,
        }
    }
}

/// Queues the job `request` needs in its current status, written in `lang`
/// (unless one is already open).
///
/// # Errors
///
/// Database failures.
pub async fn enqueue_for(
    tx: &mut Transaction<'_, Postgres>,
    request: i64,
    status: Status,
    lang: Lang,
) -> Result<(), sqlx::Error> {
    let Some(task) = Task::for_status(status) else { return Ok(()) };
    sqlx::query(
        "insert into ai_jobs (request_id, task, lang) values ($1, $2, $3)
         on conflict (request_id, task) where status in ('queued', 'running') do nothing",
    )
    .bind(request)
    .bind(task.as_str())
    .bind(lang.pick("ko", "en"))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

struct Job {
    id: i64,
    request: i64,
    task: Task,
    lang: Lang,
    attempts: i32,
}

async fn claim(db: &PgPool) -> Result<Option<Job>, sqlx::Error> {
    let row = sqlx::query(
        "update ai_jobs set status = 'running', started_at = now(), attempts = attempts + 1
          where id = (select id from ai_jobs where status = 'queued' and run_after <= now()
                       order by run_after, id for update skip locked limit 1)
          returning id, request_id, task, lang, attempts",
    )
    .fetch_optional(db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let task: String = row.try_get("task")?;
    let lang: String = row.try_get("lang")?;
    Ok(Some(Job {
        id: row.try_get("id")?,
        request: row.try_get("request_id")?,
        task: if task == "work" { Task::Work } else { Task::Report },
        lang: if lang == "en" { Lang::En } else { Lang::Ko },
        attempts: row.try_get("attempts")?,
    }))
}

async fn finish(db: impl sqlx::PgExecutor<'_>, job: i64) -> Result<(), sqlx::Error> {
    sqlx::query("update ai_jobs set status = 'done', finished_at = now() where id = $1").bind(job).execute(db).await?;
    Ok(())
}

/// Runs one due job, if there is one. Returns whether there was.
///
/// # Errors
///
/// Database failures (a provider's failure is recorded on the job instead).
pub async fn run_once<P: Provider>(db: &PgPool, provider: &P, settings: &AiSettings) -> Result<bool, sqlx::Error> {
    let Some(job) = claim(db).await? else { return Ok(false) };
    let expected = job.task.waits_in();
    let request = {
        let mut conn = db.acquire().await?;
        StoredRequest::find(&mut conn, job.request).await?
    };
    let Some(request) = request.filter(|r| r.status == expected) else {
        // Cancelled, or moved on without the AI: nothing left to do.
        finish(db, job.id).await?;
        return Ok(true);
    };

    let context = {
        let mut conn = db.acquire().await?;
        let project = Project::find(&mut *conn, &request.project_id.to_string()).await?;
        let selection = match (job.task, field(&request.details, "work").get("agent")) {
            // A work run uses the agent recorded when it started.
            (Task::Work, Some(agent)) => Selection {
                provider: agent["provider"].as_str().unwrap_or("anthropic").to_owned(),
                model: agent["model"].as_str().unwrap_or_default().to_owned(),
            },
            _ => settings::selections(&mut conn)
                .await?
                .resolve(&request.project_id.to_string(), job.task.as_str())
                .clone(),
        };
        JobContext { lang: job.lang, selection, project }
    };

    let (outcome, spend) = match job.task {
        Task::Report => {
            let reply = provider.report(&request, &context).await;
            (reply.outcome.map(Done::Report), reply.spend)
        }
        Task::Work => {
            let reply = provider.work(&request, &context).await;
            (reply.outcome.map(Done::Work), reply.spend)
        }
    };
    // Every call is booked, whatever became of it, against the person whose
    // step started the job.
    let cost_krw = book(db, &job, &request, &context.selection, &spend).await?;
    let done = match outcome {
        Ok(done) => done,
        Err(failure) => {
            let error = failure.message;
            tracing::warn!(job = job.id, request = job.request, %error, retry = failure.retry, "AI job failed");
            let (status, retry) = if failure.retry && job.attempts < MAX_ATTEMPTS {
                ("queued", job.attempts * 30)
            } else {
                ("failed", 0)
            };
            sqlx::query(
                "update ai_jobs
                    set status = $2, error = $3, run_after = now() + make_interval(secs => $4),
                        finished_at = case when $2 = 'failed' then now() end
                  where id = $1",
            )
            .bind(job.id)
            .bind(status)
            .bind(error)
            .bind(f64::from(retry))
            .execute(db)
            .await?;
            return Ok(true);
        }
    };

    let mut tx = db.begin().await?;
    let Some(mut request) =
        StoredRequest::find_for_update(&mut tx, job.request).await?.filter(|r| r.status == expected)
    else {
        finish(&mut *tx, job.id).await?;
        tx.commit().await?;
        return Ok(true);
    };
    let at = store::now_iso(&mut *tx).await?;
    let event = match done {
        Done::Report(result) => apply_report(&mut request, result, &at, settings),
        Done::Work(result) => apply_work(&mut request, result, &at, cost_krw),
    };
    let from = request.status;
    request.status = event.rule().to;
    store::add_event(&mut tx, request.id, Actor::Ai, event.as_str(), Some(from), Some(request.status), None).await?;
    request.save(&mut tx).await?;
    let owners = store::owners_of(&mut *tx, request.project_id).await?;
    notifications::notify_status(&mut tx, &request, &owners, event.as_str(), None).await?;
    finish(&mut *tx, job.id).await?;
    tx.commit().await?;
    Ok(true)
}

/// Books each spend at today's prices; returns the won booked. A model
/// without a price is logged and not booked (the frontend's `bookUsage`).
async fn book(
    db: &PgPool,
    job: &Job,
    request: &StoredRequest,
    selection: &Selection,
    spend: &[Spend],
) -> Result<f64, sqlx::Error> {
    let user = started_by(request, job.task.waits_in());
    let mut conn = db.acquire().await?;
    let mut booked = 0.0;
    for s in spend {
        let ran = Selection { provider: selection.provider.clone(), model: s.model.clone() };
        let call = Call {
            task: job.task.as_str(),
            selection: &ran,
            request_id: Some(request.id),
            user_id: user,
            tokens: s.tokens,
        };
        // The entry's business date is not used here, so any zone will do.
        match usage::book(&mut conn, "UTC", &call).await? {
            Some(entry) => booked += entry.cost_krw,
            None => tracing::warn!(model = %s.model, "AI usage of a model without a price was not booked"),
        }
    }
    Ok(booked)
}

/// The person whose step put the request where the job runs: whoever moved
/// it into `status` last; the requester when nobody did (the first report
/// starts by itself on submission). The mock API's `startedBy`.
pub fn started_by(request: &StoredRequest, status: Status) -> Uuid {
    for entry in request.timeline.iter().rev() {
        if entry["to"].as_str() != Some(status.as_str()) {
            continue;
        }
        if let Some(user) = entry["actor"]["userId"].as_str().and_then(|u| Uuid::parse_str(u).ok()) {
            return user;
        }
        break;
    }
    request.requester_id
}

enum Done {
    Report(super::ReportResult),
    Work(super::WorkResult),
}

/// Stores the report (next revision) and, for a feasible or conditional one
/// with the scope check on, the scope the requester confirms.
fn apply_report(request: &mut StoredRequest, result: super::ReportResult, at: &str, settings: &AiSettings) -> Event {
    let d = &mut request.details;
    let revision = field(d, "report")["revision"].as_i64().unwrap_or(0) + 1;
    let mut report = result.report;
    report["revision"] = json!(revision);
    report["generatedAt"] = json!(at);
    let confirm = settings.scope_check && matches!(report["verdict"].as_str(), Some("feasible" | "conditional"));
    d.insert("report".into(), report);
    match result.scope.filter(|_| confirm) {
        Some(mut proposal) => {
            let scope_revision = field(d, "scope")["proposal"]["revision"].as_i64().unwrap_or(0) + 1;
            proposal["revision"] = json!(scope_revision);
            d.insert("scope".into(), json!({ "proposal": proposal }));
            Event::ScopeReady
        }
        None => Event::ReportReady,
    }
}

/// Closes the work run with the provider's result and what it cost.
fn apply_work(request: &mut StoredRequest, result: super::WorkResult, at: &str, cost_krw: f64) -> Event {
    let d = &mut request.details;
    let mut work = field(d, "work").clone();
    if !work.is_object() {
        work = json!({});
    }
    work["phase"] = json!("finished");
    work["finishedAt"] = json!(at);
    work["outcome"] = json!(result.outcome);
    work["summary"] = json!(result.summary);
    if cost_krw > 0.0 {
        work["costKrw"] = usage::money(usage::round_money(cost_krw));
    }
    for (key, value) in result.extra {
        work[key.as_str()] = value;
    }
    d.insert("work".into(), work);
    if !result.regression.is_empty() {
        d.insert("regression".into(), Value::Array(result.regression));
    }
    Event::WorkFinished
}

/// Runs every due job; for tests and one-off runs. Returns how many ran.
///
/// # Errors
///
/// Database failures.
pub async fn run_pending<P: Provider>(db: &PgPool, provider: &P, settings: &AiSettings) -> Result<usize, sqlx::Error> {
    let mut ran = 0;
    while run_once(db, provider, settings).await? {
        ran += 1;
    }
    Ok(ran)
}

/// The background worker: returns jobs a stopped server left running to the
/// queue, then runs jobs as they become due, checking every two seconds.
pub fn spawn<P: Provider>(db: PgPool, provider: P, settings: AiSettings) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) =
            sqlx::query("update ai_jobs set status = 'queued' where status = 'running'").execute(&db).await
        {
            tracing::warn!(%error, "could not requeue unfinished AI jobs");
        }
        loop {
            match run_once(&db, &provider, &settings).await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(Duration::from_secs(2)).await,
                Err(error) => {
                    tracing::warn!(%error, "AI worker");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    })
}
