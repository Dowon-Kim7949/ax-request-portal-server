//! Helpers shared by the database tests: an app on the test's database, HTTP
//! calls, and accounts made the way `ax-api user create` makes them.
#![allow(dead_code)] // each test binary uses a different subset

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use ax_api::accounts::{self, NewAccount};
use ax_api::roles::Role;
use ax_api::users::User;
use ax_api::{AppState, Config, router};

pub const PASSWORD: &str = "correct horse battery staple";

pub fn app(pool: &PgPool) -> Router {
    let config = Config::from_lookup(|_| None).unwrap();
    router(AppState { db: Some(pool.clone()), ..AppState::default() }, &config)
}

pub async fn send(
    pool: &PgPool,
    method: &str,
    path: &str,
    token: Option<&str>,
    lang: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path).header("accept-language", lang);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app(pool).oneshot(request.header("content-type", "application/json").body(body).unwrap()).await;
    let response = response.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, body)
}

pub async fn login(pool: &PgPool, email: &str, password: &str, lang: &str) -> (StatusCode, Value) {
    send(pool, "POST", "/auth/login", None, lang, Some(json!({ "email": email, "password": password }))).await
}

/// Signs in and returns the token.
pub async fn token_for(pool: &PgPool, email: &str) -> String {
    let (status, body) = login(pool, email, PASSWORD, "en").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["token"].as_str().unwrap().to_owned()
}

pub async fn me(pool: &PgPool, token: &str) -> (StatusCode, Value) {
    send(pool, "GET", "/auth/me", Some(token), "en", None).await
}

pub async fn account(pool: &PgPool, email: &str, role: Role) -> User {
    let new =
        NewAccount { email: email.to_owned(), name: "Ada Lovelace".to_owned(), role, password: PASSWORD.to_owned() };
    accounts::create(pool, new).await.unwrap()
}

pub fn signed_out(lang: &str) -> Value {
    let message = if lang == "ko" { "다시 로그인해 주세요." } else { "Please sign in again." };
    json!({ "code": "unauthorized", "message": message })
}

/// An account with `PASSWORD`, signed in: `(user, token)`.
pub async fn signed_in(pool: &PgPool, email: &str, role: Role) -> (User, String) {
    let user = account(pool, email, role).await;
    let token = token_for(pool, email).await;
    (user, token)
}
