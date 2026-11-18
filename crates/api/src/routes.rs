//! The router: health probes, implemented operations, and a `501` for every
//! other operation in the contract.
//!
//! To implement an operation: add its `operationId` to
//! [`IMPLEMENTED`](crate::implemented::IMPLEMENTED) and an arm to [`implemented`].

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, get};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::auth;
use crate::config::Config;
use crate::error::ApiError;
use crate::implemented::IMPLEMENTED;
use crate::lang::Lang;
use crate::state::AppState;
use crate::{teams, users};

/// The whole application.
///
/// `/healthz` and `/readyz` sit at the root whatever `base_path` is, so probes
/// do not depend on how the proxy maps the API.
pub fn router(state: AppState, config: &Config) -> Router {
    let api = contract_routes().fallback(not_found);
    let probes = Router::new().route("/healthz", get(healthz)).route("/readyz", get(readyz));
    let app = if config.base_path == "/" {
        probes.merge(api)
    } else {
        probes.nest(&config.base_path, api).fallback(not_found)
    };
    app.with_state(state).layer(TraceLayer::new_for_http())
}

/// Every contract operation, on the contract's own method and path: its
/// handler if it has one, `501` otherwise.
///
/// # Panics
///
/// When [`IMPLEMENTED`] and [`implemented`] disagree about an operation. Every
/// test builds the router, so the mismatch cannot reach a release.
fn contract_routes() -> Router<AppState> {
    let mut by_path: BTreeMap<&'static str, MethodRouter<AppState>> = BTreeMap::new();
    for op in ax_contract::operations() {
        let Some(filter) = method_filter(&op.method) else { continue };
        let id: &'static str = op.id.as_str();
        let methods = by_path.remove(op.path.as_str()).unwrap_or_default();
        let listed = IMPLEMENTED.contains(&id);
        let methods = match implemented(id) {
            Some(add_handler) => {
                assert!(listed, "{id} has a handler in routes::implemented but is not listed in IMPLEMENTED");
                add_handler(methods, filter)
            }
            None => {
                assert!(!listed, "{id} is listed in IMPLEMENTED but has no handler in routes::implemented");
                methods.on(filter, move |lang: Lang| async move { ApiError::not_implemented(lang, id) })
            }
        };
        by_path.insert(op.path.as_str(), methods);
    }
    by_path.into_iter().fold(Router::new(), |router, (path, methods)| router.route(path, methods))
}

type AddHandler = fn(MethodRouter<AppState>, MethodFilter) -> MethodRouter<AppState>;

/// How to add the handler of operation `id` to its path, or `None` when it has
/// none yet. One arm per entry of [`IMPLEMENTED`]; the method and path come
/// from the contract.
fn implemented(id: &str) -> Option<AddHandler> {
    Some(match id {
        "login" => |methods, filter| methods.on(filter, auth::handlers::login),
        "logout" => |methods, filter| methods.on(filter, auth::handlers::logout),
        "me" => |methods, filter| methods.on(filter, auth::handlers::me),
        "listUsers" => |methods, filter| methods.on(filter, users::handlers::list),
        "createUser" => |methods, filter| methods.on(filter, users::handlers::create),
        "updateUser" => |methods, filter| methods.on(filter, users::handlers::update),
        "listTeams" => |methods, filter| methods.on(filter, teams::handlers::list),
        "createTeam" => |methods, filter| methods.on(filter, teams::handlers::create),
        "updateTeam" => |methods, filter| methods.on(filter, teams::handlers::update),
        _ => return None,
    })
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
