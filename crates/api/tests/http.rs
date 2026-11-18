//! The router answers the contract's shape over HTTP.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ax_api::{AppState, Config, router};

fn app(base_path: &str) -> Router {
    let config = Config::from_lookup(|name| (name == "AX_BASE_PATH").then(|| base_path.to_owned())).unwrap();
    router(AppState::default(), &config)
}

async fn call(app: Router, method: &str, path: &str, lang: &str) -> (StatusCode, Value) {
    let request =
        Request::builder().method(method).uri(path).header("accept-language", lang).body(Body::empty()).unwrap();
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
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED, "{} {} ({})", op.method, op.path, op.id);
        assert_eq!(body["code"], "internal");
        assert_eq!(body["message"], format!("Not implemented yet: {}", op.id));
    }
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
    let (status, _) = call(app("/api"), "GET", "/api/auth/me", "en").await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);

    let (status, body) = call(app("/api"), "GET", "/auth/me", "en").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "not_found");

    let (status, _) = call(app("/api"), "GET", "/healthz", "en").await;
    assert_eq!(status, StatusCode::OK);
}
