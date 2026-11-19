//! The router answers the contract's shape over HTTP.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ax_api::implemented::IMPLEMENTED;
use ax_api::{AppState, Config, router};

fn app(base_path: &str) -> Router {
    let config = Config::from_lookup(|name| (name == "AX_BASE_PATH").then(|| base_path.to_owned())).unwrap();
    router(AppState::default(), &config)
}

async fn call(app: Router, method: &str, path: &str, lang: &str) -> (StatusCode, Value) {
    send(app, Request::builder().method(method).uri(path).header("accept-language", lang).body(Body::empty()).unwrap())
        .await
}

async fn send(app: Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, body)
}

/// `/requests/{id}` → `/requests/1`.
fn concrete(path: &str) -> String {
    path.split('/').map(|segment| if segment.starts_with('{') { "1" } else { segment }).collect::<Vec<_>>().join("/")
}

#[tokio::test]
async fn health_names_the_contract() {
    let (status, body) = call(app("/"), "GET", "/healthz", "ko").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["contract"]["commit"], ax_contract::source_commit());
}

#[tokio::test]
async fn ready_without_a_database() {
    let (status, body) = call(app("/"), "GET", "/readyz", "ko").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["database"], "not_configured");
}

#[tokio::test]
async fn every_contract_operation_is_routed() {
    for op in ax_contract::operations() {
        let (status, body) = call(app("/"), &op.method, &concrete(&op.path), "en").await;
        let what = format!("{} {} ({}) answered {status}", op.method, op.path, op.id);
        if IMPLEMENTED.contains(&op.id.as_str()) {
            assert!(
                ![StatusCode::NOT_IMPLEMENTED, StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED].contains(&status),
                "{what}"
            );
        } else {
            assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{what}");
            assert_eq!(body["message"], format!("Not implemented yet: {}", op.id));
        }
        assert!(body["code"].is_string(), "{what}: the body is not an ApiError");
    }
}

#[tokio::test]
async fn signed_in_operations_need_a_bearer_token() {
    for (lang, message) in [("en", "Please sign in again."), ("ko", "다시 로그인해 주세요.")] {
        let (status, body) = call(app("/"), "GET", "/auth/me", lang).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, serde_json::json!({ "code": "unauthorized", "message": message }));
    }
    let (status, _) = call(app("/"), "POST", "/auth/logout", "en").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn without_a_database_a_token_or_a_valid_login_is_500() {
    let me = Request::get("/auth/me").header("authorization", "Bearer axs_whatever").body(Body::empty()).unwrap();
    let (status, body) = send(app("/"), me).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["code"], "internal");

    let login = Request::post("/auth/login")
        .header("accept-language", "en")
        .body(Body::from(r#"{"email":"ada@example.com","password":"x"}"#))
        .unwrap();
    let (status, body) = send(app("/"), login).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body["message"], "The server has no database configured.");
}

#[tokio::test]
async fn the_login_body_is_checked_first() {
    let login =
        Request::post("/auth/login").header("accept-language", "en").body(Body::from(r#"{"email":"x"}"#)).unwrap();
    let (status, body) = send(app("/"), login).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    assert_eq!(body["message"], "Please check the input.");
    assert_eq!(body["issues"][0]["path"], "email");
    assert_eq!(body["issues"][1]["path"], "password");
}

#[tokio::test]
async fn unknown_paths_get_the_contract_error_in_the_requested_language() {
    let (status, body) = call(app("/"), "GET", "/nope", "en").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, serde_json::json!({ "code": "not_found", "message": "Not found." }));

    let (_, body) = call(app("/"), "GET", "/nope", "ko-KR,ko;q=0.9").await;
    assert_eq!(body["message"], "요청한 주소를 찾을 수 없습니다.");
}

#[tokio::test]
async fn a_base_path_moves_the_contract_but_not_the_probes() {
    let (status, _) = call(app("/api"), "GET", "/api/requests", "en").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    let (status, _) = call(app("/api"), "GET", "/api/auth/me", "en").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, body) = call(app("/api"), "GET", "/auth/me", "en").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    let (status, _) = call(app("/api"), "GET", "/healthz", "en").await;
    assert_eq!(status, StatusCode::OK);
}
