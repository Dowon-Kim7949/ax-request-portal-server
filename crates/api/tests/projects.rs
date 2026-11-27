//! Projects against a real PostgreSQL: `listProjects`, `createProject`,
//! `updateProject`, with the frontend mock API's rules (`saveProject`).

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::roles::Role;
use common::*;

fn input(owner_ids: &[String]) -> Value {
    json!({
        "name": "  Customer portal ",
        "description": "The public web site",
        "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/web/portal" },
        "wikiPath": "docs/llm-wiki",
        "ownerIds": owner_ids,
        "targets": [{ "name": "Notices", "wikiDoc": "domains/notice.md" }, { "name": "Sign-in" }],
        "constraints": [{ "text": "No personal data in logs", "reference": "Security 3.2" }],
        "active": true,
    })
}

async fn create(pool: &PgPool, token: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "POST", "/projects", Some(token), "en", Some(body)).await
}

async fn update(pool: &PgPool, token: &str, id: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "PUT", &format!("/projects/{id}"), Some(token), "en", Some(body)).await
}

fn error(code: &str, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_admin_registers_a_project_everyone_can_list(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev, _) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;

    let (status, list) = send(&pool, "GET", "/projects", Some(&requester), "en", None).await;
    assert_eq!((status, list), (StatusCode::OK, json!([])));

    let (status, project) = create(&pool, &admin, input(&[dev.id.to_string()])).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["name"], "Customer portal", "trimmed");
    assert_eq!(project["ownerIds"], json!([dev.id.to_string()]));
    assert_eq!(project["wikiStats"], json!({ "verifiedDocs": 0, "totalDocs": 0 }));
    assert_eq!(project["repository"], json!({ "provider": "gitlab", "url": "https://git.example.com/web/portal" }));
    let targets = project["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0]["wikiDoc"], "domains/notice.md");
    assert!(targets[1].get("wikiDoc").is_none());
    assert!(targets.iter().all(|t| t["id"].as_str().is_some_and(|id| !id.is_empty())), "targets get ids");
    assert_eq!(project["constraints"][0]["reference"], "Security 3.2");
    assert!(project["constraints"][0]["id"].is_string());

    let (_, list) = send(&pool, "GET", "/projects", Some(&requester), "en", None).await;
    assert_eq!(list, json!([project]));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn owners_are_developers_or_admins_in_order(pool: PgPool) {
    let (admin_user, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let dev = account(&pool, "dev@example.com", Role::Developer).await;
    let requester = account(&pool, "req@example.com", Role::Requester).await;
    let refused = (StatusCode::BAD_REQUEST, error("bad_request", "Owners must be developers or admins."));

    for owners in [
        vec![requester.id.to_string()],
        vec![dev.id.to_string(), "00000000-0000-0000-0000-000000000000".to_owned()],
        vec!["u-kim".to_owned()],
    ] {
        assert_eq!(create(&pool, &admin, input(&owners)).await, refused, "{owners:?}");
    }

    // Repeats are dropped; the order is kept, so the first owner stays first.
    let owners = [dev.id.to_string(), admin_user.id.to_string(), dev.id.to_string()];
    let (status, project) = create(&pool, &admin, input(&owners)).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["ownerIds"], json!([dev.id.to_string(), admin_user.id.to_string()]));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn only_project_managers_change_projects(pool: PgPool) {
    let (dev_user, dev) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let owners = [dev_user.id.to_string()];

    // The body is checked before the permission, as in the mock API.
    let (status, body) = create(&pool, &dev, json!({ "name": "" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["issues"][0]["path"], "name");

    let (status, body) = create(&pool, &dev, input(&owners)).await;
    assert_eq!((status, body), (StatusCode::FORBIDDEN, error("forbidden", "You do not have permission to do this.")));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_update_replaces_the_project_but_keeps_ids_and_wiki_numbers(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let dev = account(&pool, "dev@example.com", Role::Developer).await;
    let dev2 = account(&pool, "dev2@example.com", Role::Developer).await;
    let (_, created) = create(&pool, &admin, input(&[dev.id.to_string()])).await;
    let id = created["id"].as_str().unwrap();
    sqlx::query("update projects set wiki_verified_docs = 7, wiki_total_docs = 9").execute(&pool).await.unwrap();

    let kept = created["targets"][0].clone();
    let mut body = input(&[dev2.id.to_string(), dev.id.to_string()]);
    body["name"] = json!("Portal");
    body["active"] = json!(false);
    body["targets"] = json!([kept, { "name": "Search" }]);
    body["constraints"] = json!([]);
    let (status, project) = update(&pool, &admin, id, body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{project}");
    assert_eq!(project["id"], id);
    assert_eq!(project["name"], "Portal");
    assert_eq!(project["active"], false);
    assert_eq!(project["ownerIds"], json!([dev2.id.to_string(), dev.id.to_string()]));
    assert_eq!(project["targets"][0], kept, "a target sent with its id keeps it");
    assert_ne!(project["targets"][1]["id"], kept["id"]);
    assert_eq!(project["constraints"], json!([]));
    assert_eq!(project["wikiStats"], json!({ "verifiedDocs": 7, "totalDocs": 9 }));

    let not_found = (StatusCode::NOT_FOUND, error("not_found", "The selected project was not found."));
    for unknown in ["00000000-0000-0000-0000-000000000000", "p-portal"] {
        assert_eq!(update(&pool, &admin, unknown, body.clone()).await, not_found, "{unknown}");
    }
    // An owner who is not a developer is refused before the unknown project, as in the mock API.
    body["ownerIds"] = json!(["u-kim"]);
    let (status, _) = update(&pool, &admin, "p-portal", body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_body_is_the_contracts_project_input(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let dev = account(&pool, "dev@example.com", Role::Developer).await;
    let mut body = input(&[dev.id.to_string()]);
    body["repository"]["url"] = json!("git@git.example.com:web/portal.git");
    body["constraints"] = json!(vec![json!({ "text": "rule" }); 21]);
    body["name"] = json!("x".repeat(61));
    let (status, answer) = create(&pool, &admin, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let paths: Vec<&str> = answer["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["name", "repository.url", "constraints"]);
    let count: i64 = sqlx::query_scalar("select count(*) from projects").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_project_names_the_branch_the_ai_works_from(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let dev = account(&pool, "dev@example.com", Role::Developer).await;
    let mut body = input(&[dev.id.to_string()]);
    body["repository"]["branch"] = json!(" release/2026.11 ");
    let (status, project) = create(&pool, &admin, body.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    assert_eq!(project["repository"]["branch"], "release/2026.11", "trimmed");
    let id = project["id"].as_str().unwrap().to_owned();
    let (_, list) = send(&pool, "GET", "/projects", Some(&admin), "en", None).await;
    assert_eq!(list[0]["repository"]["branch"], "release/2026.11");

    // Replacing the project without a branch goes back to the default one.
    body["repository"] = json!({ "provider": "gitlab", "url": "https://git.example.com/web/portal" });
    let (status, project) = update(&pool, &admin, &id, body.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert!(project["repository"].get("branch").is_none(), "{project}");

    for bad in [json!(""), json!("a..b"), json!("-x"), json!("has space"), json!(null), json!(7)] {
        body["repository"]["branch"] = bad.clone();
        let (status, answer) = update(&pool, &admin, &id, body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(
            answer["issues"],
            json!([{ "path": "repository.branch", "message": "This is not a valid branch name." }])
        );
    }
}
