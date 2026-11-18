//! One-time setup links against a real PostgreSQL: `createSetupLink` and
//! `completeSetup`, with the frontend mock API's rules and messages.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::auth::token;
use ax_api::roles::Role;
use common::*;

const PASSPHRASE: &str = "a brand new passphrase";

fn invalid(lang: &str) -> Value {
    let message = if lang == "ko" {
        "링크가 올바르지 않거나 이미 쓰였거나 만료되었습니다. 관리자에게 새 링크를 요청해 주세요."
    } else {
        "This link is not valid, was already used or has expired. Ask your admin for a new one."
    };
    json!({ "code": "unauthorized", "message": message })
}

async fn make_link(pool: &PgPool, admin: &str, user_id: &str) -> (StatusCode, Value) {
    send(pool, "POST", &format!("/users/{user_id}/setup-link"), Some(admin), "en", None).await
}

async fn link_token(pool: &PgPool, admin: &str, user_id: &str) -> String {
    let (status, body) = make_link(pool, admin, user_id).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body["token"].as_str().unwrap().to_owned()
}

async fn complete(pool: &PgPool, link: &str, password: &str, lang: &str) -> (StatusCode, Value) {
    send(pool, "POST", "/auth/setup", None, lang, Some(json!({ "token": link, "password": password }))).await
}

/// A person added through the API: no password yet.
async fn added(pool: &PgPool, admin: &str, email: &str) -> String {
    let body = json!({ "name": "Grace", "email": email, "role": "requester" });
    let (status, user) = send(pool, "POST", "/users", Some(admin), "en", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{user}");
    user["id"].as_str().unwrap().to_owned()
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_person_added_in_the_app_sets_their_password_and_is_signed_in(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let id = added(&pool, &admin, "grace@example.com").await;
    assert_eq!(login(&pool, "grace@example.com", PASSPHRASE, "en").await.0, StatusCode::UNAUTHORIZED);

    let (status, body) = make_link(&pool, &admin, &id).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let link = body["token"].as_str().unwrap();
    assert!(link.starts_with(token::LINK_PREFIX), "{link}");
    let expires = body["expiresAt"].as_str().unwrap();
    assert!(
        expires.len() == 24 && expires.ends_with('Z') && expires.as_bytes()[10] == b'T',
        "the contract's Timestamp: {expires}"
    );
    let hours: f64 =
        sqlx::query_scalar("select extract(epoch from expires_at - now())::float8 / 3600 from setup_links")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!((71.9..=72.0).contains(&hours), "{hours}");

    // Public: no bearer token needed.
    let (status, session) = complete(&pool, link, PASSPHRASE, "en").await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(session["user"]["id"], id.as_str());
    let new_token = session["token"].as_str().unwrap();
    assert!(new_token.starts_with(token::PREFIX));
    assert_eq!(me(&pool, new_token).await.1["email"], "grace@example.com");

    // The link is spent; the password works.
    assert_eq!(complete(&pool, link, "another long passphrase", "ko").await, (StatusCode::UNAUTHORIZED, invalid("ko")));
    assert_eq!(login(&pool, "grace@example.com", PASSPHRASE, "en").await.0, StatusCode::OK);
    let links: i64 = sqlx::query_scalar("select count(*) from setup_links").fetch_one(&pool).await.unwrap();
    assert_eq!(links, 0);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_new_link_replaces_the_old_one_and_resets_the_password(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (dev, dev_session) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let id = dev.id.to_string();

    let old = link_token(&pool, &admin, &id).await;
    let new = link_token(&pool, &admin, &id).await;
    assert_ne!(old, new);
    let stored: Vec<u8> = sqlx::query_scalar("select token_hash from setup_links").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, token::digest(&new), "only the newest link's hash is kept");
    assert_eq!(complete(&pool, &old, PASSPHRASE, "en").await, (StatusCode::UNAUTHORIZED, invalid("en")));

    // Using it resets the password and signs the person out everywhere else.
    assert_eq!(complete(&pool, &new, PASSPHRASE, "en").await.0, StatusCode::OK);
    assert_eq!(me(&pool, &dev_session).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&pool, "dev@example.com", PASSWORD, "en").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&pool, "dev@example.com", PASSPHRASE, "en").await.0, StatusCode::OK);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn links_follow_the_mock_apis_rules(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let operator = account(&pool, "ops@example.com", Role::Operator).await;
    let id = added(&pool, &admin, "grace@example.com").await;

    let (status, body) = make_link(&pool, &requester, &id).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden")));
    for unknown in ["00000000-0000-0000-0000-000000000000", "u-kim"] {
        let (status, body) = make_link(&pool, &admin, unknown).await;
        assert_eq!((status, body["message"].as_str()), (StatusCode::NOT_FOUND, Some("User not found.")), "{unknown}");
    }
    let (status, body) = make_link(&pool, &admin, &operator.id.to_string()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["message"], "Operator accounts are managed by the service and cannot be created or changed here.");

    // Deactivating deletes the unused link; a deactivated person gets none.
    let link = link_token(&pool, &admin, &id).await;
    let path = format!("/users/{id}");
    send(&pool, "PATCH", &path, Some(&admin), "en", Some(json!({ "active": false }))).await;
    let (status, body) = make_link(&pool, &admin, &id).await;
    assert_eq!(
        (status, body),
        (
            StatusCode::BAD_REQUEST,
            json!({ "code": "bad_request", "message": "Activate this person before making a link." })
        )
    );
    send(&pool, "PATCH", &path, Some(&admin), "en", Some(json!({ "active": true }))).await;
    assert_eq!(complete(&pool, &link, PASSPHRASE, "en").await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_link_for_a_deactivated_or_late_person_is_refused(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let id = added(&pool, &admin, "grace@example.com").await;

    let link = link_token(&pool, &admin, &id).await;
    sqlx::query("update setup_links set expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(complete(&pool, &link, PASSPHRASE, "en").await, (StatusCode::UNAUTHORIZED, invalid("en")));

    // Deactivated behind the API's back: the link is refused and kept, not spent.
    let link = link_token(&pool, &admin, &id).await;
    sqlx::query("update users set active = false where email = 'grace@example.com'").execute(&pool).await.unwrap();
    assert_eq!(complete(&pool, &link, PASSPHRASE, "en").await.0, StatusCode::UNAUTHORIZED);
    let password: Option<String> =
        sqlx::query_scalar("select password_hash from users where email = 'grace@example.com'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(password, None);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_body_is_the_contracts_setup_body(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let id = added(&pool, &admin, "grace@example.com").await;
    let link = link_token(&pool, &admin, &id).await;

    for (body, paths) in [
        (json!({}), vec!["token", "password"]),
        (json!({ "token": "", "password": "x".repeat(11) }), vec!["token", "password"]),
        (json!({ "token": link, "password": "x".repeat(129) }), vec!["password"]),
        (json!({ "token": link, "password": 123_456_789_012_u64 }), vec!["password"]),
    ] {
        let (status, answer) = send(&pool, "POST", "/auth/setup", None, "en", Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let got: Vec<&str> = answer["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
        assert_eq!(got, paths, "{body}");
    }
    // A refused body leaves the link usable; 12 and 128 characters are both fine.
    assert_eq!(complete(&pool, &link, &"가".repeat(12), "en").await.0, StatusCode::OK);
    let link = link_token(&pool, &admin, &id).await;
    assert_eq!(complete(&pool, &link, &"x".repeat(128), "en").await.0, StatusCode::OK);
}
