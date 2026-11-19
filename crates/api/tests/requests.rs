//! Requests against a real PostgreSQL: `createRequest`, `getRequest`,
//! `listRequests` and `precheckRequest`, with the frontend mock API's rules.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::roles::Role;
use common::*;

struct World {
    admin: String,
    owner: String,
    owner_id: String,
    other_dev: String,
    requester: String,
    requester_id: String,
    second_requester: String,
    executive: String,
    operator: String,
    project: String,
    targets: Vec<String>,
}

async fn world(pool: &PgPool) -> World {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (owner_user, owner) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (_, other_dev) = signed_in(pool, "dev2@example.com", Role::Developer).await;
    let (requester_user, requester) = signed_in(pool, "req@example.com", Role::Requester).await;
    let (_, second_requester) = signed_in(pool, "req2@example.com", Role::Requester).await;
    let (_, executive) = signed_in(pool, "exec@example.com", Role::Executive).await;
    let (_, operator) = signed_in(pool, "ops@example.com", Role::Operator).await;
    let body = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [owner_user.id.to_string()],
        "targets": [{ "name": "Notices" }, { "name": "Search" }], "constraints": [], "active": true,
    });
    let (status, project) = send(pool, "POST", "/projects", Some(&admin), "en", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    World {
        admin,
        owner,
        owner_id: owner_user.id.to_string(),
        other_dev,
        requester,
        requester_id: requester_user.id.to_string(),
        second_requester,
        executive,
        operator,
        project: project["id"].as_str().unwrap().to_owned(),
        targets: project["targets"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap().to_owned()).collect(),
    }
}

fn draft(w: &World, title: &str) -> Value {
    json!({
        "projectId": w.project, "type": "change", "title": title, "targetIds": [w.targets[0]],
        "currentBehavior": "Notices are hard to find", "desiredBehavior": "Search notices by title",
        "acceptanceCriteria": [{ "text": "When I type a word, notices with it in the title are shown" }],
        "attachmentIds": [],
    })
}

async fn file(pool: &PgPool, token: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "POST", "/requests", Some(token), "en", Some(body)).await
}

async fn list(pool: &PgPool, token: &str, query: &str) -> (StatusCode, Value) {
    send(pool, "GET", &format!("/requests{query}"), Some(token), "en", None).await
}

fn ids(list: &Value) -> Vec<i64> {
    list.as_array().unwrap().iter().map(|r| r["id"].as_i64().unwrap()).collect()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_filed_request_is_reported_on_and_goes_to_the_first_owner(pool: PgPool) {
    let w = world(&pool).await;
    let mut body = draft(&w, "  Search notices ");
    body["desiredBehavior"] = json!("Make it nicer");
    body["outOfScope"] = json!("");
    body["draftId"] = json!("d-1");
    let (status, request) = file(&pool, &w.requester, body).await;
    assert_eq!(status, StatusCode::CREATED, "{request}");

    assert_eq!(request["id"], 1);
    assert_eq!(request["title"], "Search notices");
    assert_eq!(request["status"], "reporting");
    assert_eq!(request["requesterId"], w.requester_id);
    assert_eq!(request["assigneeId"], w.owner_id);
    assert_eq!(
        request["acceptanceCriteria"],
        json!([{ "id": "c1", "text": "When I type a word, notices with it in the title are shown" }])
    );
    assert!(request.get("outOfScope").is_none(), "an empty optional is left out");
    assert!(request.get("aiUsage").is_none(), "a requester has no report:read");
    assert_eq!(request["attachments"], json!([]));
    assert_eq!((&request["regression"], &request["verification"]), (&json!([]), &json!([])));
    assert_eq!(
        request["precheck"],
        json!([{ "field": "desiredBehavior", "severity": "warning",
                 "message": "Words like \"nicer\" mean different things to different people. Say what should happen." }])
    );

    let timeline = request["timeline"].as_array().unwrap();
    assert_eq!(timeline.len(), 2);
    assert_eq!(timeline[0]["actor"], json!({ "type": "user", "userId": w.requester_id }));
    assert_eq!((&timeline[0]["event"], &timeline[0]["to"]), (&json!("created"), &json!("submitted")));
    assert!(timeline[0].get("from").is_none());
    assert_eq!(timeline[1]["actor"], json!({ "type": "system" }));
    assert_eq!(
        (&timeline[1]["event"], &timeline[1]["from"], &timeline[1]["to"]),
        (&json!("startReport"), &json!("submitted"), &json!("reporting"))
    );
    let at = request["createdAt"].as_str().unwrap();
    assert_eq!(at.len(), 24, "toISOString: {at}");
    assert_eq!((request["updatedAt"].as_str(), timeline[1]["at"].as_str()), (Some(at), Some(at)));

    // A developer (report:read) sees the AI usage, still empty.
    let (status, seen) = send(&pool, "GET", "/requests/1", Some(&w.owner), "en", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(seen["aiUsage"], json!({ "spentKrw": 0, "limitKrw": 0, "entries": [] }));
    let (_, again) = send(&pool, "GET", "/requests/1", Some(&w.requester), "en", None).await;
    assert_eq!(again, request);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn checklist_items_become_the_criteria(pool: PgPool) {
    let w = world(&pool).await;
    let mut body = draft(&w, "Audit findings");
    body["type"] = json!("checklist");
    body["acceptanceCriteria"] = json!([]);
    body["checklistSource"] = json!(" Security audit ");
    body["checklist"] = json!([
        { "ref": " F-1 ", "clause": " ", "finding": "Weak passwords", "expected": "12 characters at least" },
        { "ref": "F-2", "clause": "2.3.1", "finding": "Logs keep emails" }
    ]);
    let (status, request) = file(&pool, &w.requester, body).await;
    assert_eq!(status, StatusCode::CREATED, "{request}");
    assert_eq!(
        request["checklist"],
        json!([
            { "id": "k1", "criterionId": "c1", "ref": "F-1", "finding": "Weak passwords", "expected": "12 characters at least" },
            { "id": "k2", "criterionId": "c2", "ref": "F-2", "clause": "2.3.1", "finding": "Logs keep emails" }
        ])
    );
    assert_eq!(
        request["acceptanceCriteria"],
        json!([{ "id": "c1", "text": "[F-1] 12 characters at least" }, { "id": "c2", "text": "[F-2] Logs keep emails" }])
    );
    assert_eq!(request["checklistSource"], "Security audit");

    // checklistSource belongs to checklist requests only.
    let mut body = draft(&w, "Plain change");
    body["checklistSource"] = json!("Audit");
    let (_, request) = file(&pool, &w.requester, body).await;
    assert!(request.get("checklistSource").is_none());
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn filing_follows_the_mock_apis_rules(pool: PgPool) {
    let w = world(&pool).await;
    // The body is checked before the permission, as in the mock API.
    let (status, body) = file(&pool, &w.operator, json!({ "title": "x" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["issues"][0]["path"], "projectId");
    let (status, _) = file(&pool, &w.operator, draft(&w, "Ops wants it")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mut body = draft(&w, "Elsewhere");
    body["projectId"] = json!("p-portal");
    assert_eq!(
        file(&pool, &w.requester, body).await,
        (StatusCode::BAD_REQUEST, json!({ "code": "bad_request", "message": "The selected project was not found." }))
    );
    let mut body = draft(&w, "Unknown screen");
    body["targetIds"] = json!([w.targets[1], "t-x"]);
    assert_eq!(
        file(&pool, &w.requester, body).await,
        (StatusCode::BAD_REQUEST, json!({ "code": "bad_request", "message": "Unknown screen: t-x" }))
    );
    let mut body = draft(&w, "With files");
    body["attachmentIds"] = json!(["a-1", "a-2"]);
    let (status, answer) = file(&pool, &w.requester, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let paths: Vec<&str> = answer["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["attachmentIds.0", "attachmentIds.1"]);

    // An inactive project takes no requests.
    let (_, mut project) = send(&pool, "GET", "/projects", Some(&w.admin), "en", None).await;
    let mut project = project[0].take();
    project["active"] = json!(false);
    for key in ["id", "wikiStats"] {
        project.as_object_mut().unwrap().remove(key);
    }
    let (status, _) =
        send(&pool, "PUT", &format!("/projects/{}", w.project), Some(&w.admin), "en", Some(project)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = file(&pool, &w.requester, draft(&w, "Too late")).await;
    assert_eq!(
        (status, body["message"].as_str()),
        (StatusCode::BAD_REQUEST, Some("The selected project was not found."))
    );

    let count: i64 = sqlx::query_scalar("select count(*) from requests").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn only_those_who_may_open_a_request_see_it(pool: PgPool) {
    let w = world(&pool).await;
    file(&pool, &w.requester, draft(&w, "Mine")).await;
    for token in [&w.requester, &w.owner, &w.other_dev, &w.executive, &w.admin] {
        assert_eq!(send(&pool, "GET", "/requests/1", Some(token), "en", None).await.0, StatusCode::OK);
    }
    let hidden = json!({ "code": "not_found", "message": "Request #1 was not found." });
    assert_eq!(
        send(&pool, "GET", "/requests/1", Some(&w.second_requester), "en", None).await,
        (StatusCode::NOT_FOUND, hidden)
    );
    let (status, body) = send(&pool, "GET", "/requests/2", Some(&w.admin), "ko", None).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::NOT_FOUND, Some("요청 #2을 찾을 수 없습니다.")));
    for bad in ["abc", "0", "1.5"] {
        let (status, body) = send(&pool, "GET", &format!("/requests/{bad}"), Some(&w.admin), "en", None).await;
        assert_eq!(
            (status, body["message"].as_str(), body["issues"][0]["path"].as_str()),
            (StatusCode::NOT_FOUND, Some("Not found."), Some("id")),
            "{bad}"
        );
    }
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn lists_by_scope_status_project_and_search(pool: PgPool) {
    let w = world(&pool).await;
    file(&pool, &w.requester, draft(&w, "Search notices")).await;
    file(&pool, &w.second_requester, draft(&w, "Dark mode")).await;
    sqlx::query("update requests set status = 'review', updated_at = now() + interval '1 minute' where id = 1")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(ids(&list(&pool, &w.requester, "").await.1), [1]);
    assert_eq!(ids(&list(&pool, &w.second_requester, "?scope=mine").await.1), [2]);
    // Most recently changed first.
    assert_eq!(ids(&list(&pool, &w.executive, "?scope=all").await.1), [1, 2]);
    assert_eq!(ids(&list(&pool, &w.owner, "?scope=assigned").await.1), [1, 2]);
    assert_eq!(ids(&list(&pool, &w.other_dev, "?scope=assigned").await.1), Vec::<i64>::new());
    assert_eq!(ids(&list(&pool, &w.admin, "?scope=assigned").await.1), [1, 2]);
    assert_eq!(list(&pool, &w.requester, "?scope=all").await.0, StatusCode::FORBIDDEN);
    assert_eq!(list(&pool, &w.requester, "?scope=assigned").await.0, StatusCode::FORBIDDEN);
    let (status, body) = list(&pool, &w.requester, "?scope=everything").await;
    assert_eq!(
        (status, body["message"].as_str(), body["issues"][0]["path"].as_str()),
        (StatusCode::BAD_REQUEST, Some("The query parameters are invalid."), Some("scope"))
    );

    let all = |q: &str| format!("?scope=all&{q}");
    assert_eq!(ids(&list(&pool, &w.executive, &all("status=review")).await.1), [1]);
    assert_eq!(ids(&list(&pool, &w.executive, &all("status=reporting,nonsense")).await.1), [2]);
    assert_eq!(ids(&list(&pool, &w.executive, &all(&format!("projectId={}", w.project))).await.1), [1, 2]);
    assert_eq!(ids(&list(&pool, &w.executive, &all("projectId=p-x")).await.1), Vec::<i64>::new());
    assert_eq!(ids(&list(&pool, &w.executive, &all("q=%20SEARCH%20")).await.1), [1]);
    assert_eq!(ids(&list(&pool, &w.executive, &all("q=%232")).await.1), [2]);
    assert_eq!(ids(&list(&pool, &w.executive, &all("q=%23")).await.1), [1, 2], "a lone # filters nothing");
    // The last of a repeated parameter wins.
    assert_eq!(ids(&list(&pool, &w.executive, "?scope=mine&scope=all").await.1), [1, 2]);

    let (_, summaries) = list(&pool, &w.executive, "?scope=all").await;
    let first = summaries[0].as_object().unwrap();
    let mut keys: Vec<&str> = first.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["assigneeId", "createdAt", "id", "projectId", "requesterId", "status", "title", "type", "updatedAt"]
    );
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn precheck_advises_without_filing(pool: PgPool) {
    let w = world(&pool).await;
    let mut body = draft(&w, "Faster");
    body.as_object_mut().unwrap().remove("draftId");
    body["desiredDate"] = json!("2000-01-01");
    body["acceptanceCriteria"] = json!([{ "text": "좀 빠르게" }]);
    let (status, answer) =
        send(&pool, "POST", "/requests/precheck", Some(&w.requester), "ko", Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let fields: Vec<&str> = answer["issues"].as_array().unwrap().iter().map(|i| i["field"].as_str().unwrap()).collect();
    assert_eq!(fields, ["acceptanceCriteria.0", "desiredDate", "desiredDateReason"]);
    assert_eq!(answer["issues"][1]["message"], "희망 일정이 이미 지난 날짜입니다.");

    assert_eq!(
        send(&pool, "POST", "/requests/precheck", Some(&w.operator), "en", Some(body)).await.0,
        StatusCode::FORBIDDEN
    );
    let count: i64 = sqlx::query_scalar("select count(*) from requests").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
}
