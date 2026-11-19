//! The AI's job queue against a real PostgreSQL: jobs queued by the
//! lifecycle, run by a provider, applied as the AI's events; the stub
//! provider, scope confirmation, retries, and requests that moved on.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::ai::jobs::{MAX_ATTEMPTS, run_pending};
use ax_api::ai::{Provider, ReportResult, StubProvider, WorkResult};
use ax_api::config::AiSettings;
use ax_api::lang::Lang;
use ax_api::requests::store::StoredRequest;
use ax_api::roles::Role;
use common::*;

struct World {
    dev: String,
    requester: String,
}

async fn world(pool: &PgPool, lang: &str) -> World {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (dev_user, dev) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (_, requester) = signed_in(pool, "req@example.com", Role::Requester).await;
    let project = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [dev_user.id.to_string()],
        "targets": [], "constraints": [], "active": true,
    });
    let (_, project) = send(pool, "POST", "/projects", Some(&admin), "en", Some(project)).await;
    let request = json!({
        "projectId": project["id"], "type": "non_code", "title": "Search notices", "targetIds": [],
        "currentBehavior": "", "desiredBehavior": "Search notices by title",
        "acceptanceCriteria": [{ "text": "When I type, matching notices show" }, { "text": "When none match, a hint shows" }],
        "attachmentIds": [],
    });
    let (status, _) = send(pool, "POST", "/requests", Some(&requester), lang, Some(request)).await;
    assert_eq!(status, StatusCode::CREATED);
    World { dev, requester }
}

async fn get(pool: &PgPool, token: &str) -> Value {
    send(pool, "GET", "/requests/1", Some(token), "en", None).await.1
}

async fn jobs(pool: &PgPool) -> Vec<(String, String, i32)> {
    sqlx::query_as("select task, status, attempts from ai_jobs order by id").fetch_all(pool).await.unwrap()
}

fn settings() -> AiSettings {
    AiSettings::default()
}

async fn inbox(pool: &PgPool, token: &str) -> Vec<Value> {
    send(pool, "GET", "/notifications", Some(token), "en", None).await.1.as_array().unwrap().clone()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_stub_reports_honestly_and_the_developer_reviews(pool: PgPool) {
    let w = world(&pool, "en").await;
    assert_eq!(jobs(&pool).await, [("report".into(), "queued".into(), 0)]);
    assert_eq!(run_pending(&pool, &StubProvider, &settings()).await.unwrap(), 1);
    assert_eq!(jobs(&pool).await, [("report".into(), "done".into(), 1)]);

    let request = get(&pool, &w.dev).await;
    assert_eq!(request["status"], "review");
    let report = &request["report"];
    assert_eq!(
        (&report["revision"], &report["verdict"], &report["wikiTrust"]),
        (&json!(1), &json!("undetermined"), &json!("low"))
    );
    assert!(report["summary"].as_str().unwrap().starts_with("No AI provider is connected"));
    assert_eq!(report["verification"].as_array().unwrap().len(), 2);
    assert!(report["generatedAt"].is_string());
    let last = request["timeline"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(
        (&last["actor"], &last["event"], &last["from"], &last["to"]),
        (&json!({ "type": "ai" }), &json!("reportReady"), &json!("reporting"), &json!("review"))
    );
    assert_eq!(inbox(&pool, &w.dev).await[0]["title"], "[#1] Feasibility report is ready");
    assert_eq!(run_pending(&pool, &StubProvider, &settings()).await.unwrap(), 0, "nothing left");

    // The developer runs the work; the stub changes nothing and says so.
    let (status, _) =
        send(&pool, "POST", "/requests/1/transitions", Some(&w.dev), "ko", Some(json!({ "event": "runWork" }))).await;
    assert_eq!(status, StatusCode::OK);
    run_pending(&pool, &StubProvider, &settings()).await.unwrap();
    let request = get(&pool, &w.dev).await;
    assert_eq!(request["status"], "code_review");
    let work = &request["work"];
    assert_eq!(
        (&work["phase"], &work["outcome"], &work["attempt"], &work["branch"]),
        (&json!("finished"), &json!("error"), &json!(1), &json!("ax/request-1"))
    );
    assert!(
        work["summary"].as_str().unwrap().starts_with("AI 제공자가 연결되지 않아"),
        "in the language of whoever ran it"
    );
    assert!(work.get("mergeRequest").is_none());
    // The AI's event tells everyone concerned, the developer who ran it too.
    assert_eq!(inbox(&pool, &w.dev).await[0]["title"], "[#1] AI work is done");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_report_is_written_in_the_requesters_language(pool: PgPool) {
    let w = world(&pool, "ko").await;
    run_pending(&pool, &StubProvider, &settings()).await.unwrap();
    let request = get(&pool, &w.requester).await;
    assert!(request["report"]["summary"].as_str().unwrap().starts_with("AI 제공자가 연결되지 않아"));
    assert_eq!(request["report"]["verification"][0]["method"], "사람이 직접 확인");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_request_that_moved_on_is_left_alone(pool: PgPool) {
    let w = world(&pool, "en").await;
    let (status, _) =
        send(&pool, "POST", "/requests/1/transitions", Some(&w.requester), "en", Some(json!({ "event": "cancel" })))
            .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_pending(&pool, &StubProvider, &settings()).await.unwrap(), 1);
    assert_eq!(jobs(&pool).await, [("report".into(), "done".into(), 1)]);
    let request = get(&pool, &w.requester).await;
    assert_eq!(request["status"], "cancelled");
    assert!(request.get("report").is_none());
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_answer_gets_a_new_revision(pool: PgPool) {
    let w = world(&pool, "en").await;
    run_pending(&pool, &StubProvider, &settings()).await.unwrap();
    send(
        &pool,
        "POST",
        "/requests/1/transitions",
        Some(&w.dev),
        "en",
        Some(json!({ "event": "requestInfo", "questions": ["Which list?"] })),
    )
    .await;
    let (status, _) = send(
        &pool,
        "POST",
        "/requests/1/transitions",
        Some(&w.requester),
        "en",
        Some(json!({ "event": "provideInfo", "answer": "Notices" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(run_pending(&pool, &StubProvider, &settings()).await.unwrap(), 1);
    let request = get(&pool, &w.dev).await;
    assert_eq!((&request["status"], &request["report"]["revision"]), (&json!("review"), &json!(2)));
}

/// A provider that finds the request feasible and asks the requester one question.
struct Feasible;

impl Provider for Feasible {
    async fn report(&self, request: &StoredRequest, lang: Lang) -> Result<ReportResult, String> {
        let mut stub = StubProvider.report(request, lang).await?;
        stub.report["verdict"] = json!("feasible");
        stub.report["assumptions"] = json!([{ "id": "a1", "text": "Notices are server-side" }]);
        let scope = json!({ "summary": "Search box above the list", "changes": ["A search box"], "unchanged": ["The list"],
            "questions": [{ "id": "q1", "text": "Where?", "options": [{ "id": "o1", "label": "Top" }, { "id": "o2", "label": "Side" }] }] });
        Ok(ReportResult { scope: Some(scope), ..stub })
    }

    async fn work(&self, request: &StoredRequest, lang: Lang) -> Result<WorkResult, String> {
        StubProvider.work(request, lang).await
    }
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_feasible_report_asks_the_requester_first(pool: PgPool) {
    let w = world(&pool, "en").await;
    run_pending(&pool, &Feasible, &settings()).await.unwrap();
    let request = get(&pool, &w.requester).await;
    assert_eq!(request["status"], "scope_check");
    assert_eq!(request["scope"]["proposal"]["revision"], 1);
    assert_eq!(request["timeline"].as_array().unwrap().last().unwrap()["event"], "scopeReady");
    assert_eq!(inbox(&pool, &w.requester).await[0]["title"], "[#1] Please confirm the scope");

    // Without the scope check, the same report goes straight to the developer.
    let (status, _) = send(
        &pool,
        "POST",
        "/requests/1/transitions",
        Some(&w.requester),
        "en",
        Some(json!({ "event": "reviseScope", "note": "Smaller" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    run_pending(&pool, &Feasible, &AiSettings { scope_check: false, ..settings() }).await.unwrap();
    let request = get(&pool, &w.requester).await;
    assert_eq!((&request["status"], &request["report"]["revision"]), (&json!("review"), &json!(2)));
}

/// A provider that always fails.
struct Broken;

impl Provider for Broken {
    async fn report(&self, _: &StoredRequest, _: Lang) -> Result<ReportResult, String> {
        Err("the model is down".into())
    }

    async fn work(&self, _: &StoredRequest, _: Lang) -> Result<WorkResult, String> {
        Err("the model is down".into())
    }
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_failing_job_is_retried_then_given_up(pool: PgPool) {
    let w = world(&pool, "en").await;
    for attempt in 1..=MAX_ATTEMPTS {
        sqlx::query("update ai_jobs set run_after = now()").execute(&pool).await.unwrap();
        assert_eq!(run_pending(&pool, &Broken, &settings()).await.unwrap(), 1);
        let (_, status, attempts) = jobs(&pool).await.remove(0);
        assert_eq!(attempts, attempt);
        assert_eq!(status, if attempt < MAX_ATTEMPTS { "queued" } else { "failed" });
    }
    let error: Option<String> = sqlx::query_scalar("select error from ai_jobs").fetch_one(&pool).await.unwrap();
    assert_eq!(error.as_deref(), Some("the model is down"));
    // The request waits for a person; nothing else runs.
    assert_eq!(get(&pool, &w.requester).await["status"], "reporting");
    sqlx::query("update ai_jobs set run_after = now()").execute(&pool).await.unwrap();
    assert_eq!(run_pending(&pool, &Broken, &settings()).await.unwrap(), 0);
}
