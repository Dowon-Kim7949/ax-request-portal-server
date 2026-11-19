//! Saved drafts against a real PostgreSQL, with the frontend mock API's rules.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use ax_api::roles::Role;
use common::*;

async fn call(pool: &PgPool, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    send(pool, method, path, Some(token), "en", body).await
}

async fn create(pool: &PgPool, token: &str, body: Value) -> Value {
    let (status, draft) = call(pool, "POST", "/drafts", token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{draft}");
    draft
}

/// An upload of `owner`'s, made directly in the table.
async fn attachment(pool: &PgPool, owner: Uuid, id: &str) {
    sqlx::query("insert into attachments (id, owner_id, name, size, mime_type) values ($1, $2, $3, 3, 'image/png')")
        .bind(id)
        .bind(owner)
        .bind(format!("{id}.png"))
        .execute(pool)
        .await
        .unwrap();
}

fn ids(list: &Value) -> Vec<&str> {
    list.as_array().unwrap().iter().map(|d| d["id"].as_str().unwrap()).collect()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_draft_is_saved_continued_and_deleted(pool: PgPool) {
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let content = json!({ "title": "  Search notices ", "type": "change", "projectId": "", "unknown": 1 });
    let draft = create(&pool, &token, json!({ "step": 1, "content": content })).await;
    let id = draft["id"].as_str().unwrap();
    assert!(id.starts_with("drf_"), "{draft}");
    assert_eq!(draft["title"], "Search notices");
    assert_eq!(draft["type"], "change");
    assert!(draft.get("projectId").is_none(), "no project picked yet");
    assert_eq!((draft["step"].as_i64(), draft["version"].as_i64()), (Some(1), Some(1)));
    assert_eq!(draft["content"], json!({ "title": "  Search notices ", "type": "change", "projectId": "" }));
    assert_eq!((draft["attachments"].clone(), draft["attachmentCount"].as_i64()), (json!([]), Some(0)));
    assert_eq!(draft["createdAt"], draft["updatedAt"]);

    let path = format!("/drafts/{id}");
    let (status, got) = call(&pool, "GET", &path, &token, None).await;
    assert_eq!((status, &got), (StatusCode::OK, &draft));

    let update = json!({ "step": 3, "content": { "title": "Search notices by title" }, "baseVersion": 1 });
    let (status, updated) = call(&pool, "PUT", &path, &token, Some(update.clone())).await;
    assert_eq!(status, StatusCode::OK, "{updated}");
    assert_eq!((updated["version"].as_i64(), updated["step"].as_i64()), (Some(2), Some(3)));
    assert_eq!(updated["content"], json!({ "title": "Search notices by title" }), "replaced, not merged");
    assert_eq!(updated["createdAt"], draft["createdAt"]);
    assert!(updated.get("type").is_none());

    // Saved meanwhile elsewhere: the old base version is refused.
    assert_eq!(
        call(&pool, "PUT", &path, &token, Some(update)).await,
        (
            StatusCode::CONFLICT,
            json!({ "code": "conflict", "message": "This draft was saved somewhere else in the meantime. Load the saved version, or save what is on this screen over it." })
        )
    );

    let (status, list) = call(&pool, "GET", "/drafts", &token, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0].get("content").is_none(), "a list shows summaries");
    assert_eq!(list[0]["version"], 2);

    let (status, body) = call(&pool, "DELETE", &path, &token, None).await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));
    let gone = json!({ "code": "not_found", "message": "The saved draft was not found. It may have been deleted or submitted on another device." });
    assert_eq!(call(&pool, "GET", &path, &token, None).await, (StatusCode::NOT_FOUND, gone.clone()));
    assert_eq!(call(&pool, "DELETE", &path, &token, None).await, (StatusCode::NOT_FOUND, gone));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn drafts_are_listed_most_recently_changed_first(pool: PgPool) {
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let first = create(&pool, &token, json!({ "step": 0, "content": { "title": "first" } })).await;
    let second = create(&pool, &token, json!({ "step": 0, "content": { "title": "second" } })).await;
    let (_, list) = call(&pool, "GET", "/drafts", &token, None).await;
    assert_eq!(ids(&list), [second["id"].as_str().unwrap(), first["id"].as_str().unwrap()]);

    let path = format!("/drafts/{}", first["id"].as_str().unwrap());
    let (status, _) =
        call(&pool, "PUT", &path, &token, Some(json!({ "step": 1, "content": {}, "baseVersion": 1 }))).await;
    assert_eq!(status, StatusCode::OK);
    let (_, list) = call(&pool, "GET", "/drafts", &token, None).await;
    assert_eq!(ids(&list), [first["id"].as_str().unwrap(), second["id"].as_str().unwrap()]);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_repeated_create_answers_with_the_first_draft(pool: PgPool) {
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (_, other) = signed_in(&pool, "req2@example.com", Role::Requester).await;
    let draft = create(&pool, &token, json!({ "step": 0, "content": { "title": "a" }, "clientId": "c-1" })).await;
    let path = format!("/drafts/{}", draft["id"].as_str().unwrap());
    let (_, updated) =
        call(&pool, "PUT", &path, &token, Some(json!({ "step": 2, "content": { "title": "b" }, "baseVersion": 1 })))
            .await;

    // The first answer was lost; the retry gets the draft as it is now.
    let again = create(&pool, &token, json!({ "step": 5, "content": { "title": "z" }, "clientId": "c-1" })).await;
    assert_eq!(again, updated);
    // A client id is the caller's own: someone else's makes their own draft.
    let theirs = create(&pool, &other, json!({ "step": 0, "content": {}, "clientId": "c-1" })).await;
    assert_ne!(theirs["id"], draft["id"]);
    let count: i64 = sqlx::query_scalar("select count(*) from drafts").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_account_keeps_at_most_twenty_drafts(pool: PgPool) {
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    for i in 0..20 {
        create(&pool, &token, json!({ "step": 0, "content": {}, "clientId": format!("c-{i}") })).await;
    }
    assert_eq!(
        call(&pool, "POST", "/drafts", &token, Some(json!({ "step": 0, "content": {} }))).await,
        (
            StatusCode::CONFLICT,
            json!({ "code": "limit_reached", "message": "Your saved drafts are full (up to 20). Delete one to save another." })
        )
    );
    let (_, body) = send(&pool, "POST", "/drafts", Some(&token), "ko", Some(json!({ "step": 0, "content": {} }))).await;
    assert_eq!(body["message"], "임시 저장함이 가득 찼습니다(최대 20개). 다른 초안을 지우면 저장할 수 있습니다.");
    // A repeated create is not refused for the limit.
    create(&pool, &token, json!({ "step": 0, "content": {}, "clientId": "c-3" })).await;
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn drafts_hold_only_the_callers_attachments(pool: PgPool) {
    let (me, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (them, _) = signed_in(&pool, "req2@example.com", Role::Requester).await;
    attachment(&pool, me.id, "att_mine").await;
    attachment(&pool, me.id, "att_also").await;
    attachment(&pool, them.id, "att_theirs").await;

    let (status, body) = call(
        &pool,
        "POST",
        "/drafts",
        &token,
        Some(json!({ "step": 4, "content": { "attachmentIds": ["att_mine", "att_theirs", "att_gone"] } })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "An attachment was not found. Remove it and upload it again.");
    let paths: Vec<&str> = body["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["content.attachmentIds.1", "content.attachmentIds.2"]);

    let ids = json!(["att_also", "att_mine", "att_also"]);
    let draft = create(&pool, &token, json!({ "step": 4, "content": { "attachmentIds": ids } })).await;
    assert_eq!(draft["content"]["attachmentIds"], ids, "kept as sent");
    assert_eq!(draft["attachmentCount"], 2);
    let names: Vec<&str> =
        draft["attachments"].as_array().unwrap().iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["att_also.png", "att_mine.png"], "each once, in order");
    assert!(draft["attachments"][0]["url"].as_str().unwrap().starts_with("/api/files/att_also?exp="));
    let (_, list) = call(&pool, "GET", "/drafts", &token, None).await;
    assert_eq!(list[0]["attachmentCount"], 2);

    // An update is checked the same way, after the version.
    let path = format!("/drafts/{}", draft["id"].as_str().unwrap());
    let bad = json!({ "step": 4, "content": { "attachmentIds": ["att_theirs"] }, "baseVersion": 1 });
    let (status, body) = call(&pool, "PUT", &path, &token, Some(bad.clone())).await;
    assert_eq!(
        (status, body["issues"][0]["path"].as_str()),
        (StatusCode::BAD_REQUEST, Some("content.attachmentIds.0"))
    );
    let mut stale = bad;
    stale["baseVersion"] = json!(9);
    assert_eq!(call(&pool, "PUT", &path, &token, Some(stale)).await.0, StatusCode::CONFLICT);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn drafts_follow_the_mock_apis_order_and_stay_private(pool: PgPool) {
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (_, other) = signed_in(&pool, "req2@example.com", Role::Requester).await;
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let draft = create(&pool, &token, json!({ "step": 0, "content": {} })).await;
    let path = format!("/drafts/{}", draft["id"].as_str().unwrap());

    assert_eq!(send(&pool, "GET", "/drafts", None, "en", None).await, (StatusCode::UNAUTHORIZED, signed_out("en")));
    // The body is checked before the permission.
    let (status, body) = call(&pool, "POST", "/drafts", &operator, Some(json!({ "step": 7, "clientId": "" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let paths: Vec<&str> = body["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["step", "content", "clientId"]);
    let (status, _) = call(&pool, "POST", "/drafts", &operator, Some(json!({ "step": 0, "content": {} }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(call(&pool, "GET", "/drafts", &operator, None).await.0, StatusCode::FORBIDDEN);
    let (status, body) = call(&pool, "PUT", &path, &token, Some(json!({ "step": 0, "content": {} }))).await;
    assert_eq!((status, body["issues"][0]["path"].as_str()), (StatusCode::BAD_REQUEST, Some("baseVersion")));

    // Someone else's draft is as good as missing, and stays as it was.
    let update = json!({ "step": 6, "content": { "title": "mine now" }, "baseVersion": 1 });
    assert_eq!(call(&pool, "GET", &path, &other, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "PUT", &path, &other, Some(update)).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "DELETE", &path, &other, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&pool, "GET", "/drafts", &other, None).await.1, json!([]));
    assert_eq!(call(&pool, "GET", &path, &token, None).await.1, draft);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn filing_a_request_deletes_its_draft(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (owner, _) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let (_, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (_, other) = signed_in(&pool, "req2@example.com", Role::Requester).await;
    let project = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [owner.id.to_string()],
        "targets": [], "constraints": [], "active": true,
    });
    let (_, project) = call(&pool, "POST", "/projects", &admin, Some(project)).await;
    let mine = create(&pool, &token, json!({ "step": 6, "content": { "title": "Show errors" } })).await;
    let theirs = create(&pool, &other, json!({ "step": 6, "content": {} })).await;
    let request = |draft_id: &Value| {
        json!({
            "projectId": project["id"], "type": "change", "title": "Show errors", "targetIds": [],
            "currentBehavior": "", "desiredBehavior": "Say what went wrong",
            "acceptanceCriteria": [{ "text": "When saving fails, the reason is shown" }], "attachmentIds": [],
            "draftId": draft_id,
        })
    };

    // Someone else's draft id is ignored, and their draft stays.
    let (status, _) = call(&pool, "POST", "/requests", &token, Some(request(&theirs["id"]))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(call(&pool, "GET", "/drafts", &other, None).await.1.as_array().unwrap().len(), 1);

    let (status, _) = call(&pool, "POST", "/requests", &token, Some(request(&mine["id"]))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(call(&pool, "GET", "/drafts", &token, None).await.1, json!([]));
}
