//! People and teams against a real PostgreSQL: `listUsers`, `createUser`,
//! `updateUser`, `listTeams`, `createTeam`, `updateTeam`, with the frontend
//! mock API's rules and messages.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::roles::Role;
use common::*;

fn error(code: &str, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

fn issue_paths(body: &Value) -> Vec<&str> {
    body["issues"]
        .as_array()
        .map(|issues| issues.iter().filter_map(|i| i["path"].as_str()).collect())
        .unwrap_or_default()
}

async fn post(pool: &PgPool, path: &str, token: &str, lang: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "POST", path, Some(token), lang, Some(body)).await
}

async fn patch(pool: &PgPool, path: &str, token: &str, lang: &str, body: Value) -> (StatusCode, Value) {
    send(pool, "PATCH", path, Some(token), lang, Some(body)).await
}

async fn get(pool: &PgPool, path: &str, token: &str) -> (StatusCode, Value) {
    send(pool, "GET", path, Some(token), "en", None).await
}

async fn team(pool: &PgPool, token: &str, name: &str) -> String {
    let (status, body) = post(pool, "/teams", token, "en", json!({ "name": name })).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["id"].as_str().unwrap().to_owned()
}

/* ---------- users ---------- */

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn anyone_signed_in_lists_everyone(pool: PgPool) {
    let (admin, _) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (requester, token) = signed_in(&pool, "req@example.com", Role::Requester).await;
    sqlx::query("update users set active = false where email = 'admin@example.com'").execute(&pool).await.unwrap();

    let (status, body) = get(&pool, "/users", &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body,
        json!([
            { "id": admin.id.to_string(), "name": "Ada Lovelace", "email": "admin@example.com", "role": "admin", "active": false },
            { "id": requester.id.to_string(), "name": "Ada Lovelace", "email": "req@example.com", "role": "requester", "active": true },
        ]),
        "oldest first, deactivated people included, no teamId key without a team"
    );

    let (status, _) = send(&pool, "GET", "/users", None, "en", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_admin_adds_people(pool: PgPool) {
    let (_, token) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let team_id = team(&pool, &token, "Planning").await;

    let (status, body) = post(
        &pool,
        "/users",
        &token,
        "en",
        json!({ "name": "  Grace Hopper ", "email": "Grace@Example.com", "role": "developer", "teamId": team_id }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["name"], "Grace Hopper");
    assert_eq!(body["email"], "Grace@Example.com");
    assert_eq!(body["role"], "developer");
    assert_eq!(body["teamId"], team_id.as_str());
    assert_eq!(body["active"], true);

    let (_, list) = get(&pool, "/users", &token).await;
    assert_eq!(list.as_array().unwrap().len(), 2);

    // No password yet: the account cannot sign in.
    let (status, _) = login(&pool, "grace@example.com", PASSWORD, "en").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Without a team, the key is left out.
    let (status, body) = post(
        &pool,
        "/users",
        &token,
        "en",
        json!({ "name": "Alan", "email": "alan@example.com", "role": "requester" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(body.get("teamId").is_none(), "{body}");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn adding_people_follows_the_mock_apis_rules(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let ok = json!({ "name": "Grace", "email": "grace@example.com", "role": "developer" });

    // The body is checked before the permission, as in the mock API.
    let (status, body) =
        post(&pool, "/users", &requester, "en", json!({ "name": " ", "role": "root", "teamId": 5 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "Please check the input.");
    assert_eq!(issue_paths(&body), ["name", "email", "role", "teamId"]);

    let (status, body) = post(&pool, "/users", &requester, "en", ok.clone()).await;
    assert_eq!((status, body), (StatusCode::FORBIDDEN, error("forbidden", "You do not have permission to do this.")));

    let operator = json!({ "name": "Op", "email": "op@example.com", "role": "operator" });
    let (status, body) = post(&pool, "/users", &admin, "ko", operator).await;
    assert_eq!(
        (status, body),
        (
            StatusCode::FORBIDDEN,
            error("forbidden", "운영자 계정은 서비스가 관리합니다. 여기서 만들거나 바꿀 수 없습니다.")
        )
    );

    let taken = json!({ "name": "Twin", "email": "REQ@example.com", "role": "requester" });
    let (status, body) = post(&pool, "/users", &admin, "ko", taken).await;
    assert_eq!((status, body), (StatusCode::BAD_REQUEST, error("bad_request", "이미 등록된 이메일입니다.")));

    for team in ["00000000-0000-0000-0000-000000000000", "t-planning", ""] {
        let input = json!({ "name": "Grace", "email": "grace@example.com", "role": "developer", "teamId": team });
        let (status, body) = post(&pool, "/users", &admin, "en", input).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error("not_found", "Team not found.")), "{team:?}");
    }

    // A taken email wins over an unknown team, as in the mock API.
    let both = json!({ "name": "Twin", "email": "req@example.com", "role": "requester", "teamId": "t-x" });
    assert_eq!(post(&pool, "/users", &admin, "en", both).await.1["message"], "This email is already registered.");

    let count: i64 = sqlx::query_scalar("select count(*) from users").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 2);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_admin_changes_role_team_and_active(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev, dev_token) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let team_id = team(&pool, &admin, "Engineering").await;
    let path = format!("/users/{}", dev.id);

    let (status, body) = patch(&pool, &path, &admin, "en", json!({ "role": "executive", "teamId": team_id })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["role"].as_str(), body["teamId"].as_str()), (Some("executive"), Some(team_id.as_str())));

    // Absent and "" keep the team (the mock API's reading); null takes the person out.
    assert_eq!(patch(&pool, &path, &admin, "en", json!({})).await.1["teamId"], team_id.as_str());
    assert_eq!(patch(&pool, &path, &admin, "en", json!({ "teamId": "" })).await.1["teamId"], team_id.as_str());
    let (_, body) = patch(&pool, &path, &admin, "en", json!({ "teamId": null })).await;
    assert!(body.get("teamId").is_none(), "{body}");
    assert_eq!(body["role"], "executive");

    // Deactivating ends their sessions at once.
    assert_eq!(get(&pool, "/auth/me", &dev_token).await.0, StatusCode::OK);
    let (_, body) = patch(&pool, &path, &admin, "en", json!({ "active": false })).await;
    assert_eq!(body["active"], false);
    assert_eq!(get(&pool, "/auth/me", &dev_token).await.0, StatusCode::UNAUTHORIZED);
    let sessions: i64 = sqlx::query_scalar("select count(*) from sessions where user_id = $1")
        .bind(dev.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 0);

    let (_, body) = patch(&pool, &path, &admin, "en", json!({ "active": true })).await;
    assert_eq!(body["active"], true);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn changing_people_follows_the_mock_apis_rules(pool: PgPool) {
    let (me, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev, requester_token) = signed_in(&pool, "dev@example.com", Role::Requester).await;
    let operator = account(&pool, "ops@example.com", Role::Operator).await;
    let dev_path = format!("/users/{}", dev.id);
    let my_path = format!("/users/{}", me.id);

    let (status, body) =
        patch(&pool, &dev_path, &admin, "en", json!({ "role": "boss", "active": null, "teamId": 1 })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(issue_paths(&body), ["role", "teamId", "active"]);

    let (status, _) = patch(&pool, &dev_path, &requester_token, "en", json!({ "role": "admin" })).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    for id in ["00000000-0000-0000-0000-000000000000", "u-kim"] {
        let (status, body) = patch(&pool, &format!("/users/{id}"), &admin, "en", json!({})).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error("not_found", "User not found.")), "{id}");
    }

    let provisioned = "Operator accounts are managed by the service and cannot be created or changed here.";
    let (status, body) =
        patch(&pool, &format!("/users/{}", operator.id), &admin, "en", json!({ "active": false })).await;
    assert_eq!((status, body), (StatusCode::FORBIDDEN, error("forbidden", provisioned)));
    let (status, body) = patch(&pool, &dev_path, &admin, "en", json!({ "role": "operator" })).await;
    assert_eq!((status, body), (StatusCode::FORBIDDEN, error("forbidden", provisioned)));

    let own = error("bad_request", "자신의 역할이나 활성 상태는 바꿀 수 없습니다.");
    assert_eq!(
        patch(&pool, &my_path, &admin, "ko", json!({ "active": false })).await,
        (StatusCode::BAD_REQUEST, own.clone())
    );
    assert_eq!(
        patch(&pool, &my_path, &admin, "ko", json!({ "role": "developer" })).await,
        (StatusCode::BAD_REQUEST, own)
    );
    // Their own team, or their own role and status unchanged, are fine.
    assert_eq!(
        patch(&pool, &my_path, &admin, "en", json!({ "role": "admin", "active": true })).await.0,
        StatusCode::OK
    );
    let team_id = team(&pool, &admin, "Operations").await;
    assert_eq!(patch(&pool, &my_path, &admin, "en", json!({ "teamId": team_id })).await.0, StatusCode::OK);

    for team in ["00000000-0000-0000-0000-000000000000", "t-x"] {
        let (status, body) = patch(&pool, &dev_path, &admin, "en", json!({ "teamId": team })).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error("not_found", "Team not found.")), "{team}");
    }
}

/* ---------- teams ---------- */

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn teams_are_listed_by_name(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    for name in ["sales", "Engineering", "  Planning  ", "기획"] {
        team(&pool, &admin, name).await;
    }
    let (status, body) = get(&pool, "/teams", &requester).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body.as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Engineering", "Planning", "sales", "기획"]);
    assert!(body[0]["id"].is_string());
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn team_names_are_unique_ignoring_case(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let planning = team(&pool, &admin, "Planning").await;
    let sales = team(&pool, &admin, "Sales").await;

    let taken = error("bad_request", "같은 이름의 팀이 이미 있습니다.");
    assert_eq!(
        post(&pool, "/teams", &admin, "ko", json!({ "name": " planning " })).await,
        (StatusCode::BAD_REQUEST, taken.clone())
    );

    for name in [json!(""), json!("   "), json!("x".repeat(41)), json!(7), Value::Null] {
        let (status, body) = post(&pool, "/teams", &admin, "en", json!({ "name": name })).await;
        assert_eq!((status, issue_paths(&body)), (StatusCode::BAD_REQUEST, vec!["name"]), "{name}");
    }
    assert_eq!(post(&pool, "/teams", &admin, "en", json!({ "name": "x".repeat(40) })).await.0, StatusCode::CREATED);
    assert_eq!(post(&pool, "/teams", &requester, "en", json!({ "name": "Mine" })).await.0, StatusCode::FORBIDDEN);

    // Renaming: to another team's name is refused, a change of case of its own is not.
    let path = format!("/teams/{sales}");
    assert_eq!(
        patch(&pool, &path, &admin, "ko", json!({ "name": "PLANNING" })).await,
        (StatusCode::BAD_REQUEST, taken)
    );
    let (status, body) = patch(&pool, &path, &admin, "en", json!({ "name": "SALES " })).await;
    assert_eq!((status, body), (StatusCode::OK, json!({ "id": sales, "name": "SALES" })));
    assert_eq!(
        patch(&pool, &format!("/teams/{planning}"), &requester, "en", json!({ "name": "P" })).await.0,
        StatusCode::FORBIDDEN
    );

    for id in ["00000000-0000-0000-0000-000000000000", "t-planning"] {
        let (status, body) = patch(&pool, &format!("/teams/{id}"), &admin, "en", json!({ "name": "New" })).await;
        assert_eq!((status, body), (StatusCode::NOT_FOUND, error("not_found", "Team not found.")), "{id}");
    }
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn people_follow_a_renamed_team(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let team_id = team(&pool, &admin, "Planning").await;
    let (_, body) = post(
        &pool,
        "/users",
        &admin,
        "en",
        json!({ "name": "Grace", "email": "grace@example.com", "role": "requester", "teamId": team_id }),
    )
    .await;
    let user_id = body["id"].as_str().unwrap().to_owned();

    patch(&pool, &format!("/teams/{team_id}"), &admin, "en", json!({ "name": "Strategy" })).await;
    let (_, users) = get(&pool, "/users", &admin).await;
    let grace = users.as_array().unwrap().iter().find(|u| u["id"] == user_id.as_str()).unwrap();
    assert_eq!(grace["teamId"], team_id.as_str());
    let (_, teams) = get(&pool, "/teams", &admin).await;
    assert_eq!(teams, json!([{ "id": team_id, "name": "Strategy" }]));
}
