//! `listRepositoryBranches` against a fake GitHub: the mock API's order
//! (session, query, permission, repository) and what GitHub's answers become.

mod common;

use std::collections::HashMap;

use axum::extract::{Path, Query};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::config::{GitHubRead, Secret};
use ax_api::projects::branches::{MAX_PAGES, PER_PAGE};
use ax_api::roles::Role;
use ax_api::{AppState, Config, router};
use common::*;

const READ_TOKEN: &str = "ghp_read_only";

fn signed(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(format!("Bearer {READ_TOKEN}").as_str())
}

/// GitHub's repository and branch API: `acme/portal` has 102 branches over
/// two pages, `acme/huge` more than the server reads, and any other
/// repository is unknown. Every call must carry the read token.
async fn github() -> String {
    let about = |headers: HeaderMap, Path((owner, name)): Path<(String, String)>| async move {
        if !signed(&headers) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        match (owner.as_str(), name.as_str()) {
            ("acme", "portal") => Ok(Json(json!({ "default_branch": "main" }))),
            ("acme", "huge") => Ok(Json(json!({ "default_branch": "trunk" }))),
            _ => Err(StatusCode::NOT_FOUND),
        }
    };
    let branches = |headers: HeaderMap,
                    Path((_owner, name)): Path<(String, String)>,
                    Query(q): Query<HashMap<String, String>>| async move {
        if !signed(&headers) || q.get("per_page").map(String::as_str) != Some("100") {
            return Err(StatusCode::BAD_REQUEST);
        }
        let page: usize = q.get("page").and_then(|p| p.parse().ok()).unwrap_or(1);
        let names: Vec<String> = match (name.as_str(), page) {
            ("portal", 1) => (0..PER_PAGE).map(|i| format!("feature/{i:03}")).collect(),
            ("portal", 2) => vec!["main".into(), "dev".into()],
            ("portal", _) => Vec::new(),
            ("huge", _) => (0..PER_PAGE).map(|i| format!("p{page:02}/{i:03}")).collect(),
            _ => return Err(StatusCode::NOT_FOUND),
        };
        Ok(Json(Value::from(names.into_iter().map(|n| json!({ "name": n })).collect::<Vec<_>>())))
    };
    let app =
        Router::new().route("/repos/{owner}/{name}", get(about)).route("/repos/{owner}/{name}/branches", get(branches));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn app(pool: &PgPool, github: GitHubRead) -> Router {
    let state = AppState { db: Some(pool.clone()), files: files(), github, ..AppState::default() };
    router(state, &Config::from_lookup(|_| None).unwrap())
}

async fn branches(app: Router, token: Option<&str>, provider: &str, url: &str) -> (StatusCode, Value) {
    let query: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("provider", provider)
        .append_pair("url", url)
        .finish();
    send_to(app, "GET", &format!("/repositories/branches?{query}"), token, "en", None).await
}

fn not_found() -> (StatusCode, Value) {
    (
        StatusCode::NOT_FOUND,
        json!({ "code": "not_found", "message": "The repository was not found, or the server cannot reach it." }),
    )
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_project_manager_lists_a_github_repositorys_branches(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let github = GitHubRead { token: Some(Secret::new(READ_TOKEN)), api: github().await };
    let with = |github: GitHubRead| app(&pool, github);
    let portal = "https://github.com/acme/portal";

    let (status, _) = branches(with(github.clone()), None, "github", portal).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // The query is checked before the permission, as in the mock API.
    let (status, answer) = branches(with(github.clone()), Some(&requester), "svn", "javascript:alert(1)").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let paths: Vec<&str> = answer["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["provider", "url"]);
    let (status, _) = branches(with(github.clone()), Some(&requester), "github", portal).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, list) = branches(with(github.clone()), Some(&admin), "github", &format!("{portal}.git")).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!((&list["defaultBranch"], &list["truncated"]), (&json!("main"), &json!(false)));
    let names = list["branches"].as_array().unwrap();
    assert_eq!(names.len(), PER_PAGE + 2);
    assert_eq!(&names[..3], &[json!("main"), json!("dev"), json!("feature/000")], "the default first, then by name");

    let (status, list) = branches(with(github.clone()), Some(&admin), "github", "https://github.com/acme/huge").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(list["truncated"], true, "more than {MAX_PAGES} pages");
    assert_eq!(list["branches"][0], "trunk");
    assert_eq!(list["branches"].as_array().unwrap().len(), MAX_PAGES * PER_PAGE + 1);

    // Whatever the server cannot read is one answer; the form takes a typed name.
    for (provider, url) in [
        ("github", "https://github.com/acme/secret"),
        ("github", "https://github.com/acme"),
        ("gitlab", "https://gitlab.example.com/acme/portal"),
    ] {
        assert_eq!(branches(with(github.clone()), Some(&admin), provider, url).await, not_found(), "{url}");
    }
    let wrong = GitHubRead { token: Some(Secret::new("ghp_other")), ..github.clone() };
    assert_eq!(branches(with(wrong), Some(&admin), "github", portal).await, not_found(), "a refused token");
    let unreachable = GitHubRead { api: "http://127.0.0.1:9".to_owned(), ..github.clone() };
    assert_eq!(branches(with(unreachable), Some(&admin), "github", portal).await, not_found(), "no answer");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn without_a_read_token_no_branches_are_read(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let github = GitHubRead { token: None, api: github().await };
    let portal = "https://github.com/acme/portal";
    assert_eq!(branches(app(&pool, github), Some(&admin), "github", portal).await, not_found());
}
