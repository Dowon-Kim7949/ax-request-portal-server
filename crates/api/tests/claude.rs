//! The Claude provider against a local fake of the Messages API: what it
//! sends, how it turns the answer into a report, what it books, and how it
//! fails.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::ai::claude::ClaudeProvider;
use ax_api::ai::jobs::run_pending;
use ax_api::config::{AiSettings, Secret, TaskKeys};
use ax_api::roles::Role;
use common::*;

type Seen = Arc<Mutex<Vec<(HeaderMap, Value)>>>;

#[derive(Clone)]
struct Fake {
    seen: Seen,
    replies: Arc<Mutex<VecDeque<(u16, Value)>>>,
}

async fn messages(State(fake): State<Fake>, headers: HeaderMap, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    fake.seen.lock().unwrap().push((headers, body));
    let (status, reply) = fake.replies.lock().unwrap().pop_front().expect("a reply for every call");
    (StatusCode::from_u16(status).unwrap(), Json(reply))
}

/// A fake API on a free local port answering `replies` in turn.
async fn fake_api(replies: Vec<(u16, Value)>) -> (ClaudeProvider, Seen) {
    let fake = Fake { seen: Arc::default(), replies: Arc::new(Mutex::new(replies.into())) };
    let app = Router::new().route("/v1/messages", post(messages)).with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let keys = TaskKeys { report: Some(Secret::new("sk-report-key-1234")), ..TaskKeys::default() };
    (ClaudeProvider::with_base_url(keys, &url), fake.seen)
}

fn answered(model: &str, answer: &Value) -> Value {
    json!({
        "id": "msg_1", "type": "message", "role": "assistant", "model": model,
        "content": [{ "type": "thinking", "thinking": "", "signature": "x" }, { "type": "text", "text": answer.to_string() }],
        "stop_reason": "end_turn", "stop_details": null,
        "usage": { "input_tokens": 2000, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 0, "output_tokens": 1500 },
    })
}

struct World {
    requester_id: String,
    rule_id: String,
}

/// A project with three screens and a rule, and request #1 (`kind`) on its
/// first screen, filed in English: a report job is queued.
async fn world(pool: &PgPool, kind: &str) -> World {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (dev, _) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (requester, token) = signed_in(pool, "req@example.com", Role::Requester).await;
    let project = json!({
        "name": "Portal", "description": "The company's notice board", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [dev.id.to_string()],
        "targets": [{ "name": "Notices" }, { "name": "Search" }, { "name": "Settings" }],
        "constraints": [{ "text": "Do not change the public API", "reference": "Rules 1" }], "active": true,
    });
    let (status, project) = send(pool, "POST", "/projects", Some(&admin), "en", Some(project)).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let request = json!({
        "projectId": project["id"], "type": kind, "title": "Search notices by title",
        "targetIds": [project["targets"][0]["id"]], "currentBehavior": "Notices are hard to find",
        "desiredBehavior": "Search notices by title",
        "acceptanceCriteria": [{ "text": "When I type a word, matching notices show" }, { "text": "When none match, a hint shows" }],
        "attachmentIds": [],
    });
    let (status, body) = send(pool, "POST", "/requests", Some(&token), "en", Some(request)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    World {
        requester_id: requester.id.to_string(),
        rule_id: project["constraints"][0]["id"].as_str().unwrap().to_owned(),
    }
}

async fn request(pool: &PgPool) -> Value {
    let admin = token_for(pool, "admin@example.com").await;
    send(pool, "GET", "/requests/1", Some(&admin), "en", None).await.1
}

async fn job(pool: &PgPool) -> (String, i32, Option<String>) {
    sqlx::query_as("select status, attempts, error from ai_jobs order by id desc limit 1")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn claude_writes_a_report_that_claims_nothing_it_did_not_read(pool: PgPool) {
    let w = world(&pool, "change").await;
    let answer = json!({
        "summary": "Asks for a title search on the notice list.",
        "verification": [
            { "criterionId": "c2", "automatable": true, "method": "An end-to-end test searches for a missing word" },
            { "criterionId": "c9", "automatable": true, "method": "Not a criterion" },
        ],
        "openQuestions": ["Should the search include the body?", " Should the search include the body? "],
        "assumptions": ["The notice list is loaded from the server"],
        "constraintChecks": [{ "constraintId": w.rule_id, "conflict": true, "note": "A search parameter changes the API" }],
        "recheck": ["Search", "Notices", "Elsewhere"],
        "size": "S",
        "risks": ["Slow searches on long lists"],
    });
    let (claude, seen) = fake_api(vec![(200, answered("claude-opus-5-5", &answer))]).await;
    assert_eq!(run_pending(&pool, &claude, &AiSettings::default()).await.unwrap(), 1);

    let request = request(&pool).await;
    assert_eq!(request["status"], "review", "an undetermined report goes to the developer");
    let report = &request["report"];
    assert_eq!(report["verdict"], "undetermined", "no evidence was read, so no verdict is claimed");
    assert_eq!((&report["plan"], &report["wikiTrust"], &report["size"]), (&json!([]), &json!("low"), &json!("S")));
    assert_eq!(report["evidence"], json!({ "wikiDocs": [], "codeFiles": [] }));
    assert_eq!(report["impact"], json!({ "changedScreens": ["Notices"], "recheck": ["Search"] }));
    assert_eq!(
        report["verification"],
        json!([
            { "criterionId": "c1", "automatable": false, "method": "Checked by hand" },
            { "criterionId": "c2", "automatable": true, "method": "An end-to-end test searches for a missing word" },
        ])
    );
    assert_eq!(report["openQuestions"], json!(["Should the search include the body?"]));
    assert_eq!(report["assumptions"], json!([{ "id": "a1", "text": "The notice list is loaded from the server" }]));
    assert_eq!(
        report["constraintChecks"],
        json!([{ "constraintId": w.rule_id, "text": "Do not change the public API", "reference": "Rules 1", "conflict": true, "note": "A search parameter changes the API" }])
    );
    assert_eq!(report["generatedBy"], json!({ "provider": "anthropic", "model": "claude-opus-5-5" }));
    assert_eq!(report["revision"], 1);

    let calls = seen.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let (headers, body) = &calls[0];
    assert_eq!(headers["x-api-key"], "sk-report-key-1234", "the report task's own key");
    assert_eq!(headers["anthropic-version"], "2026-10-21");
    assert_eq!(headers["anthropic-beta"], "server-side-fallback-2026-07-01");
    assert_eq!((&body["model"], &body["fallbacks"]), (&json!("claude-opus-5-5"), &json!("default")));
    assert_eq!(body["output_config"]["effort"], "high");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["system"][0]["cache_control"], json!({ "type": "ephemeral" }));
    assert!(body.get("thinking").is_none() && body.get("temperature").is_none());
    let prompt = body["messages"][0]["content"].as_str().unwrap();
    assert!(prompt.starts_with("Write every text field in English."), "{prompt}");
    assert!(prompt.contains("Search notices by title") && prompt.contains("Do not change the public API"));

    // The call is booked against the requester, at Opus prices:
    // 2,000 in × $4 + 1,000 cached × $0.2 + 1,500 out × $20 per million = $0.0382.
    let (user, model, input, cache_read, cost_usd, cost_krw): (String, String, i64, i64, f64, f64) = sqlx::query_as(
        "select user_id::text, model, input_tokens, cache_read_tokens, cost_usd, cost_krw from ai_usage where request_id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((user, model.as_str(), input, cache_read), (w.requester_id, "claude-opus-5-5", 2000, 1000));
    assert_eq!((cost_usd, cost_krw), (0.04, 53.48));
    assert_eq!(request["aiUsage"]["spentKrw"], 53.48);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_refusal_or_a_rejected_call_is_not_retried(pool: PgPool) {
    world(&pool, "change").await;
    let refusal = json!({
        "id": "msg_2", "type": "message", "role": "assistant", "model": "claude-opus-5-5", "content": [],
        "stop_reason": "refusal", "stop_details": { "type": "refusal", "category": "cyber", "explanation": "" },
        "usage": { "input_tokens": 900, "output_tokens": 0 },
    });
    let (claude, _) = fake_api(vec![(200, refusal)]).await;
    run_pending(&pool, &claude, &AiSettings::default()).await.unwrap();
    let (status, attempts, error) = job(&pool).await;
    assert_eq!((status.as_str(), attempts), ("failed", 1), "a decline would only repeat");
    assert_eq!(error.as_deref(), Some("Claude declined to write the report (cyber)"));
    let booked: i64 = sqlx::query_scalar("select count(*) from ai_usage").fetch_one(&pool).await.unwrap();
    assert_eq!(booked, 1, "the declined call is still booked");
    assert_eq!(request(&pool).await["status"], "reporting", "the request waits for a person");

    // A rate limit is retried later; a rejected key is not.
    sqlx::query("update ai_jobs set status = 'queued', attempts = 0, run_after = now()").execute(&pool).await.unwrap();
    let limited = json!({ "type": "error", "error": { "type": "rate_limit_error", "message": "Slow down" } });
    let rejected =
        json!({ "type": "error", "error": { "type": "authentication_error", "message": "invalid x-api-key" } });
    let (claude, _) = fake_api(vec![(429, limited), (401, rejected)]).await;
    run_pending(&pool, &claude, &AiSettings::default()).await.unwrap();
    let (status, _, error) = job(&pool).await;
    assert_eq!(status, "queued");
    assert_eq!(error.as_deref(), Some("Claude API 429 Too Many Requests rate_limit_error: Slow down"));
    sqlx::query("update ai_jobs set run_after = now()").execute(&pool).await.unwrap();
    run_pending(&pool, &claude, &AiSettings::default()).await.unwrap();
    let (status, attempts, error) = job(&pool).await;
    assert_eq!((status.as_str(), attempts), ("failed", 2));
    assert_eq!(error.as_deref(), Some("Claude API 401 Unauthorized authentication_error: invalid x-api-key"));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn haiku_takes_no_fallback_and_a_non_code_request_is_out_of_scope(pool: PgPool) {
    world(&pool, "non_code").await;
    let selections = json!({
        "defaults": {
            "precheck": { "provider": "anthropic", "model": "claude-haiku-5-5" },
            "report": { "provider": "anthropic", "model": "claude-haiku-5-5" },
            "work": { "provider": "anthropic", "model": "claude-opus-5-5" },
        },
        "projectOverrides": [],
    });
    sqlx::query("insert into ai_config (name, value) values ('selections', $1)")
        .bind(selections)
        .execute(&pool)
        .await
        .unwrap();
    let answer = json!({
        "summary": "This asks for a data export, not a code change.", "verification": [], "openQuestions": [],
        "assumptions": ["An export exists"], "constraintChecks": [], "recheck": [], "size": "XL", "risks": [],
    });
    let (claude, seen) = fake_api(vec![(200, answered("claude-haiku-5-5", &answer))]).await;
    run_pending(&pool, &claude, &AiSettings::default()).await.unwrap();

    let (headers, body) = seen.lock().unwrap()[0].clone();
    assert_eq!(body["model"], "claude-haiku-5-5");
    assert!(body.get("fallbacks").is_none() && headers.get("anthropic-beta").is_none());
    let report = request(&pool).await["report"].clone();
    assert_eq!(report["verdict"], "out_of_scope");
    assert_eq!((&report["assumptions"], &report["constraintChecks"]), (&json!([]), &json!([])));
    assert_eq!(report["size"], "M", "an unknown size falls back to M");
}
