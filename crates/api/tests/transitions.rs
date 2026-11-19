//! The request lifecycle against a real PostgreSQL: `transitionRequest`, the
//! requester's and developer's records on a request, the regression queue,
//! and the notifications all of this sends. The AI's steps do not exist yet,
//! so tests put requests where the AI would leave them with SQL.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::roles::Role;
use common::*;

struct World {
    admin: String,
    dev: String,
    other_dev: String,
    requester: String,
    requester_id: String,
    stranger: String,
}

/// A project owned by `dev`, and request #1 filed by `requester`, now `reporting`.
async fn world(pool: &PgPool) -> World {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (dev_user, dev) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (_, other_dev) = signed_in(pool, "dev2@example.com", Role::Developer).await;
    let (requester_user, requester) = signed_in(pool, "req@example.com", Role::Requester).await;
    let (_, stranger) = signed_in(pool, "req2@example.com", Role::Requester).await;
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
    let (status, body) = send(pool, "POST", "/requests", Some(&requester), "en", Some(request)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    World { admin, dev, other_dev, requester, requester_id: requester_user.id.to_string(), stranger }
}

/// Puts request #1 where the AI would leave it: a status, and `details` merged in.
async fn set(pool: &PgPool, status: &str, details: Value) {
    sqlx::query("update requests set status = $1, details = details || $2 where id = 1")
        .bind(status)
        .bind(details)
        .execute(pool)
        .await
        .unwrap();
}

async fn act(pool: &PgPool, token: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "POST", "/requests/1/transitions", Some(token), "en", Some(body)).await
}

async fn put(pool: &PgPool, token: &str, what: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "PUT", &format!("/requests/1/{what}"), Some(token), "en", Some(body)).await
}

async fn inbox(pool: &PgPool, token: &str) -> Vec<Value> {
    let (status, list) = send(pool, "GET", "/notifications", Some(token), "en", None).await;
    assert_eq!(status, StatusCode::OK);
    list.as_array().unwrap().clone()
}

fn checks(results: &[&str]) -> Value {
    let checks: Vec<Value> =
        results.iter().enumerate().map(|(i, r)| json!({ "criterionId": format!("c{}", i + 1), "result": r })).collect();
    Value::Array(checks)
}

fn last_event(request: &Value) -> Value {
    request["timeline"].as_array().unwrap().last().unwrap().clone()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn from_review_to_release(pool: PgPool) {
    let w = world(&pool).await;
    set(&pool, "review", json!({ "report": { "assumptions": [{ "id": "a1", "text": "The list is server-side" }] } }))
        .await;

    // Whose step comes before the status and the guards.
    let (status, body) = act(&pool, &w.requester, json!({ "event": "runWork" })).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::FORBIDDEN, Some("You cannot do this at this step.")));
    let (status, body) = act(&pool, &w.other_dev, json!({ "event": "runWork" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "not an owner: {body}");
    let (status, body) =
        send(&pool, "POST", "/requests/1/transitions", Some(&w.dev), "ko", Some(json!({ "event": "runWork" }))).await;
    assert_eq!(
        (status, body),
        (
            StatusCode::CONFLICT,
            json!({ "code": "guard_failed", "message": "보고서의 가정을 모두 확인해야 AI 작업을 시작할 수 있습니다." })
        )
    );

    let (status, request) = act(
        &pool,
        &w.dev,
        json!({ "event": "runWork", "checkedAssumptions": ["a1"], "instructions": " Keep it small " }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(request["status"], "in_progress");
    let work = &request["work"];
    assert_eq!(
        (&work["attempt"], &work["phase"], &work["branch"]),
        (&json!(1), &json!("queued"), &json!("ax/request-1"))
    );
    assert_eq!((&work["instructions"], &work["checkedAssumptions"]), (&json!("Keep it small"), &json!(["a1"])));
    let entry = last_event(&request);
    assert_eq!(
        (&entry["event"], &entry["from"], &entry["to"], &entry["note"]),
        (&json!("runWork"), &json!("review"), &json!("in_progress"), &json!("Keep it small"))
    );
    let mail = inbox(&pool, &w.requester).await;
    assert_eq!(
        (mail[0]["title"].as_str(), mail[0]["body"].as_str()),
        (Some("[#1] Work has started"), Some("Search notices"))
    );
    assert!(inbox(&pool, &w.dev).await.is_empty(), "the actor is not told");

    // The AI opens the merge request; the developer approves the merge.
    set(&pool, "code_review", json!({})).await;
    let (status, request) = act(&pool, &w.dev, json!({ "event": "approveMerge" })).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(
        request["verification"],
        json!([{ "criterionId": "c1", "result": "pending" }, { "criterionId": "c2", "result": "pending" }])
    );
    assert_eq!(inbox(&pool, &w.requester).await[0]["body"], "Check the 2 acceptance criteria one by one.");

    // The requester checks the result.
    let (status, request) = put(&pool, &w.requester, "verification", json!({ "checks": [{ "criterionId": "c1", "result": "passed", "note": "fine" }, { "criterionId": "x", "result": "failed" }] })).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(
        request["verification"],
        json!([{ "criterionId": "c1", "result": "passed", "note": "fine" }, { "criterionId": "c2", "result": "pending" }])
    );
    assert_eq!(put(&pool, &w.dev, "verification", json!({ "checks": [] })).await.0, StatusCode::FORBIDDEN);
    let (status, body) =
        act(&pool, &w.requester, json!({ "event": "confirmResult", "checks": checks(&["passed", "failed"]) })).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::CONFLICT, Some("Every acceptance criterion must pass before you confirm."))
    );
    let (status, body) =
        act(&pool, &w.requester, json!({ "event": "reportIssue", "checks": checks(&["passed", "passed"]) })).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::CONFLICT, Some("Mark at least one acceptance criterion as not working."))
    );
    let (status, request) =
        act(&pool, &w.requester, json!({ "event": "confirmResult", "checks": checks(&["passed", "passed"]) })).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("release_scheduled")));
    assert_eq!(inbox(&pool, &w.dev).await[0]["title"], "[#1] Set a release date");
    assert_eq!(put(&pool, &w.dev, "verification", json!({ "checks": [] })).await.0, StatusCode::FORBIDDEN);

    // The developer plans and marks the release.
    let (status, body) = act(&pool, &w.dev, json!({ "event": "markReleased" })).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::CONFLICT, Some("guard_failed")));
    let (status, body) = put(&pool, &w.dev, "release", json!({ "scheduledDate": "2000-01-03" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().unwrap().starts_with("The release date cannot be in the past. Today is "),
        "{body}"
    );
    let (status, request) =
        put(&pool, &w.dev, "release", json!({ "scheduledDate": "2099-12-30", "note": "Tuesday window" })).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(request["release"], json!({ "scheduledDate": "2099-12-30", "note": "Tuesday window" }));
    let entry = last_event(&request);
    assert_eq!((&entry["event"], &entry["note"]), (&json!("release_updated"), &json!("2099-12-30")));
    assert!(entry.get("from").is_none());
    assert_eq!(inbox(&pool, &w.requester).await[0]["body"], "Planned to go live on Wed, Dec 30.");
    assert_eq!(put(&pool, &w.requester, "release", json!({ "note": "x" })).await.0, StatusCode::FORBIDDEN);

    let (status, request) = act(&pool, &w.dev, json!({ "event": "markReleased" })).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("done")));
    assert!(request["release"]["releasedAt"].is_string());
    assert_eq!(inbox(&pool, &w.requester).await[0]["title"], "[#1] It is live");
    assert_eq!(put(&pool, &w.dev, "release", json!({ "note": "late" })).await.0, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn questions_answers_and_rejection(pool: PgPool) {
    let w = world(&pool).await;
    set(&pool, "review", json!({})).await;
    let (status, request) =
        act(&pool, &w.dev, json!({ "event": "requestInfo", "questions": ["Which list?", " Which order? "] })).await;
    assert_eq!(status, StatusCode::OK, "{request}");
    assert_eq!(request["infoRequest"]["questions"], json!(["Which list?", "Which order?"]));
    assert_eq!(last_event(&request)["note"], "Which list?\nWhich order?");
    assert_eq!(inbox(&pool, &w.requester).await[0]["body"], "Which list? / Which order?");

    let (status, request) =
        act(&pool, &w.requester, json!({ "event": "provideInfo", "answer": "The notice list, newest first" })).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("reporting")));
    assert_eq!(request["infoRequest"]["answer"], "The notice list, newest first");
    assert_eq!(inbox(&pool, &w.dev).await[0]["body"], "The report is being rewritten.");

    set(&pool, "review", json!({})).await;
    let (status, request) = act(&pool, &w.dev, json!({ "event": "reject", "reason": "Out of scope" })).await;
    assert_eq!(
        (status, &request["status"], &request["rejectionReason"]),
        (StatusCode::OK, &json!("rejected"), &json!("Out of scope"))
    );
    assert_eq!(inbox(&pool, &w.requester).await[0]["body"], "Out of scope");
    let (status, body) = act(&pool, &w.requester, json!({ "event": "cancel" })).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::CONFLICT, Some("This is not possible at the current step."))
    );
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn scope_answers_and_rewrites(pool: PgPool) {
    let w = world(&pool).await;
    let scope = json!({ "scope": { "proposal": { "revision": 1, "summary": "s", "changes": [], "unchanged": [],
        "questions": [
            { "id": "q1", "text": "Where?", "options": [{ "id": "o1", "label": "Top" }, { "id": "o2", "label": "Side" }] },
            { "id": "q2", "text": "Case?", "options": [{ "id": "o3", "label": "Any" }] }
        ] } } });
    set(&pool, "scope_check", scope.clone()).await;
    let (status, body) = act(
        &pool,
        &w.requester,
        json!({ "event": "approveScope", "choices": [{ "questionId": "q1", "optionId": "o2" }] }),
    )
    .await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::CONFLICT, Some("Answer every question first.")));
    let (status, body) = act(&pool, &w.requester, json!({ "event": "approveScope", "choices": [{ "questionId": "q1", "optionId": "o9" }, { "questionId": "q2", "optionId": "o3" }] })).await;
    assert_eq!(status, StatusCode::CONFLICT, "an option the question does not have: {body}");

    for _ in 0..2 {
        let (status, request) = act(&pool, &w.requester, json!({ "event": "reviseScope", "note": "Simpler" })).await;
        assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("reporting")));
        set(&pool, "scope_check", json!({})).await;
    }
    let (status, body) = act(&pool, &w.requester, json!({ "event": "reviseScope", "note": "Again" })).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::CONFLICT, Some("guard_failed")));

    let choices = json!([{ "questionId": "q2", "optionId": "o3" }, { "questionId": "q1", "optionId": "o1" }, { "questionId": "q1", "optionId": "o2" }]);
    let (status, request) =
        act(&pool, &w.requester, json!({ "event": "approveScope", "choices": choices, "note": " ok " })).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("review")), "{request}");
    let decision = &request["scope"]["decision"];
    assert_eq!(
        decision["choices"],
        json!([{ "questionId": "q1", "optionId": "o2" }, { "questionId": "q2", "optionId": "o3" }]),
        "question order, the last answer"
    );
    assert_eq!((&decision["by"], &decision["note"]), (&json!(w.requester_id), &json!("ok")));
    assert_eq!(inbox(&pool, &w.dev).await[0]["title"], "[#1] The requester confirmed the scope");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn who_may_cancel_and_what_is_refused(pool: PgPool) {
    let w = world(&pool).await;
    assert_eq!(act(&pool, &w.dev, json!({ "event": "cancel" })).await.0, StatusCode::FORBIDDEN);
    let (status, body) = act(&pool, &w.stranger, json!({ "event": "cancel" })).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::NOT_FOUND, Some("Request #1 was not found.")));
    let (status, body) = act(&pool, &w.requester, json!({ "event": "workFinished" })).await;
    assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("event")));
    assert_eq!(
        act(&pool, &w.requester, json!({ "event": "approveScope", "choices": [] })).await.0,
        StatusCode::CONFLICT
    );

    // An admin may cancel someone else's request; the requester and the developers are told.
    let (status, request) = act(&pool, &w.admin, json!({ "event": "cancel", "reason": "Duplicate of #2" })).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("cancelled")));
    assert_eq!(last_event(&request)["note"], "Duplicate of #2");
    assert_eq!(inbox(&pool, &w.dev).await[0]["title"], "[#1] Request cancelled");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn preview_feedback_and_regression(pool: PgPool) {
    let w = world(&pool).await;
    set(&pool, "code_review", json!({ "work": { "attempt": 1, "startedAt": "2026-11-19T00:00:00.000Z", "phase": "finished", "branch": "ax/request-1" } })).await;
    let feedback = json!({ "verdict": "needs_change", "note": " Bigger text " });
    let (status, body) = put(&pool, &w.requester, "preview-feedback", feedback.clone()).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::CONFLICT, Some("There is no preview to comment on right now."))
    );

    set(&pool, "code_review", json!({ "work": { "attempt": 1, "startedAt": "2026-11-19T00:00:00.000Z", "phase": "finished", "branch": "ax/request-1", "preview": { "url": "/preview.svg" } } })).await;
    assert_eq!(put(&pool, &w.dev, "preview-feedback", feedback.clone()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        put(&pool, &w.requester, "preview-feedback", json!({ "verdict": "needs_change" })).await.0,
        StatusCode::BAD_REQUEST
    );
    let (status, request) = put(&pool, &w.requester, "preview-feedback", feedback).await;
    assert_eq!((status, &request["status"]), (StatusCode::OK, &json!("code_review")), "{request}");
    assert_eq!(request["work"]["previewFeedback"]["verdict"], "needs_change");
    assert_eq!(request["work"]["previewFeedback"]["note"], "Bigger text");
    let entry = last_event(&request);
    assert_eq!((&entry["event"], &entry["note"]), (&json!("preview_needs_change"), &json!("Bigger text")));
    let mail = inbox(&pool, &w.dev).await;
    assert_eq!(
        (mail[0]["title"].as_str(), mail[0]["body"].as_str()),
        (Some("[#1] The requester asked for a change in the preview"), Some("Bigger text"))
    );

    // Regression checks.
    set(&pool, "verification", json!({ "regression": [{ "id": "r1", "label": "Sign-in", "source": "wiki", "result": "pending" }, { "id": "r2", "label": "Home", "source": "wiki", "result": "pending" }] })).await;
    assert_eq!(put(&pool, &w.requester, "regression", json!({ "checks": [] })).await.0, StatusCode::FORBIDDEN);
    let (status, request) = put(
        &pool,
        &w.dev,
        "regression",
        json!({ "checks": [{ "id": "r2", "result": "passed" }, { "id": "zz", "result": "failed" }] }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(request["regression"][1]["result"], "passed");
    assert_eq!(request["regression"][0]["result"], "pending");

    let queue = |token: String| {
        let pool = pool.clone();
        async move { send(&pool, "GET", "/regression-checks", Some(&token), "en", None).await }
    };
    let (status, list) = queue(w.dev.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!((list[0]["requestId"].as_i64(), list[0]["checks"].as_array().map(Vec::len)), (Some(1), Some(2)));
    assert_eq!(queue(w.other_dev.clone()).await.1, json!([]));
    assert_eq!(queue(w.admin.clone()).await.1.as_array().unwrap().len(), 1);
    assert_eq!(queue(w.requester.clone()).await.0, StatusCode::FORBIDDEN);

    set(&pool, "done", json!({})).await;
    let (status, body) = put(&pool, &w.dev, "regression", json!({ "checks": [] })).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::CONFLICT, Some("This request is closed and can no longer be changed."))
    );
    assert_eq!(queue(w.dev.clone()).await.1, json!([]));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn notifications_are_read_and_follow_settings(pool: PgPool) {
    let w = world(&pool).await;
    let (_, settings) = send(&pool, "GET", "/me/notification-settings", Some(&w.requester), "en", None).await;
    assert_eq!(settings, json!({ "inApp": true, "email": true, "messenger": false, "onlyActionRequired": false }));

    set(&pool, "review", json!({})).await;
    act(&pool, &w.dev, json!({ "event": "requestInfo", "questions": ["Which list?"] })).await;
    let mail = inbox(&pool, &w.requester).await;
    assert_eq!(mail.len(), 1);
    let id = mail[0]["id"].as_str().unwrap().to_owned();
    assert!(mail[0].get("readAt").is_none());

    let (status, read) =
        send(&pool, "POST", &format!("/notifications/{id}/read"), Some(&w.requester), "ko", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(read["readAt"].is_string());
    assert_eq!(read["title"], "[#1] 추가 정보가 필요합니다", "written in the reader's language");
    let (status, body) = send(&pool, "POST", &format!("/notifications/{id}/read"), Some(&w.dev), "en", None).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::NOT_FOUND, Some("Notification not found.")));
    assert_eq!(
        send(&pool, "POST", "/notifications/read-all", Some(&w.requester), "en", None).await.0,
        StatusCode::NO_CONTENT
    );

    // Only what needs me: a notification that asks for nothing is not kept.
    let only = json!({ "inApp": true, "email": false, "messenger": false, "onlyActionRequired": true });
    let (status, saved) =
        send(&pool, "PUT", "/me/notification-settings", Some(&w.requester), "en", Some(only.clone())).await;
    assert_eq!((status, &saved), (StatusCode::OK, &only));
    act(&pool, &w.requester, json!({ "event": "provideInfo", "answer": "This one" })).await;
    set(&pool, "review", json!({})).await;
    act(&pool, &w.dev, json!({ "event": "runWork" })).await; // workStarted asks for nothing
    assert_eq!(inbox(&pool, &w.requester).await.len(), 1);
    let (status, body) =
        send(&pool, "PUT", "/me/notification-settings", Some(&w.requester), "en", Some(json!({ "inApp": true }))).await;
    assert_eq!((status, body["issues"].as_array().map(Vec::len)), (StatusCode::BAD_REQUEST, Some(3)));
    let _ = w.requester_id;
}
