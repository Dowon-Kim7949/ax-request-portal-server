//! The router: health probes, implemented operations, and a `501` for every
//! other operation in the contract.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, get};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::error::ApiError;
use crate::implemented::IMPLEMENTED;
use crate::lang::Lang;
use crate::state::AppState;

/// The whole application.
///
/// `/healthz` and `/readyz` sit at the root whatever `base_path` is, so probes
/// do not depend on how the proxy maps the API.
pub fn router(state: AppState, config: &Config) -> Router {
    let api = implemented_routes().merge(pending_routes()).fallback(not_found);
    let probes = Router::new().route("/healthz", get(healthz)).route("/readyz", get(readyz));
    let app = if config.base_path == "/" {
        probes.merge(api)
    } else {
        probes.nest(&config.base_path, api).fallback(not_found)
    };
    app.with_state(state).layer(TraceLayer::new_for_http())
}

/// Operations with a real handler. Empty until the first one lands.
fn implemented_routes() -> Router<AppState> {
    Router::new()
}

/// Every contract operation not listed in [`IMPLEMENTED`], answering `501`.
fn pending_routes() -> Router<AppState> {
    let mut by_path: BTreeMap<&'static str, MethodRouter<AppState>> = BTreeMap::new();
    for op in ax_contract::operations() {
        if IMPLEMENTED.contains(&op.id.as_str()) {
            continue;
        }
        let Some(filter) = method_filter(&op.method) else { continue };
        let id: &'static str = op.id.as_str();
        let handler = move |lang: Lang| async move { ApiError::not_implemented(lang, id) };
        let methods = by_path.remove(op.path.as_str()).unwrap_or_default();
        by_path.insert(op.path.as_str(), methods.on(filter, handler));
    }
    by_path.into_iter().fold(Router::new(), |router, (path, methods)| router.route(path, methods))
}

fn method_filter(method: &str) -> Option<MethodFilter> {
    Some(match method {
        "GET" => MethodFilter::GET,
        "POST" => MethodFilter::POST,
        "PUT" => MethodFilter::PUT,
        "PATCH" => MethodFilter::PATCH,
        "DELETE" => MethodFilter::DELETE,
        _ => return None,
    })
}

async fn not_found(lang: Lang) -> ApiError {
    ApiError::not_found(lang)
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "contract": { "version": ax_contract::version(), "commit": ax_contract::source_commit() },
    }))
}

async fn readyz(State(state): State<AppState>) -> Response {
    let Some(pool) = &state.db else {
        return Json(json!({ "status": "ok", "database": "not_configured" })).into_response();
    };
    match sqlx::query("select 1").execute(pool).await {
        Ok(_) => Json(json!({ "status": "ok", "database": "ok" })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "database is not reachable");
            (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "status": "unavailable", "database": "unreachable" })))
                .into_response()
        }
    }
}
