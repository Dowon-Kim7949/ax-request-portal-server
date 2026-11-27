//! The router: health probes, implemented operations, and a `501` for every
//! other operation in the contract.
//!
//! To implement an operation: add its `operationId` to
//! [`IMPLEMENTED`](crate::implemented::IMPLEMENTED) and an arm to [`implemented`].
//!
//! A few routes are not in the contract: `GET /files/{id}`, the signed
//! download link that every `Attachment.url` points at (see
//! [`crate::attachments`]), and the runner's `/runner/...` (see
//! [`crate::ai::runner`]), which answer `404` unless `AX_RUNNER_TOKEN` is set.

use std::collections::BTreeMap;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodFilter, MethodRouter, get, post};
use axum::{Json, Router};
use serde_json::json;
use tower_http::trace::TraceLayer;

use crate::config::Config;
use crate::error::ApiError;
use crate::implemented::IMPLEMENTED;
use crate::lang::Lang;
use crate::state::AppState;
use crate::{ai, attachments, auth};
use crate::{dashboard, drafts, notifications, projects, requests, teams, users};

/// The whole application.
///
/// `/healthz` and `/readyz` sit at the root whatever `base_path` is, so probes
/// do not depend on how the proxy maps the API.
pub fn router(state: AppState, config: &Config) -> Router {
    let api = contract_routes()
        .route("/files/{id}", get(attachments::handlers::download))
        .route("/runner/claim", post(ai::runner::claim))
        .route("/runner/jobs/{id}/heartbeat", post(ai::runner::heartbeat))
        .route("/runner/jobs/{id}/result", post(ai::runner::result))
        .fallback(not_found);
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
        "completeSetup" => |methods, filter| methods.on(filter, auth::setup::complete),
        "createSetupLink" => |methods, filter| methods.on(filter, auth::setup::create),
        "listUsers" => |methods, filter| methods.on(filter, users::handlers::list),
        "createUser" => |methods, filter| methods.on(filter, users::handlers::create),
        "updateUser" => |methods, filter| methods.on(filter, users::handlers::update),
        "listTeams" => |methods, filter| methods.on(filter, teams::handlers::list),
        "createTeam" => |methods, filter| methods.on(filter, teams::handlers::create),
        "updateTeam" => |methods, filter| methods.on(filter, teams::handlers::update),
        "listProjects" => |methods, filter| methods.on(filter, projects::handlers::list),
        "createProject" => |methods, filter| methods.on(filter, projects::handlers::create),
        "updateProject" => |methods, filter| methods.on(filter, projects::handlers::update),
        "listRepositoryBranches" => |methods, filter| methods.on(filter, projects::branches::list),
        "listRequests" => |methods, filter| methods.on(filter, requests::handlers::list),
        "createRequest" => |methods, filter| methods.on(filter, requests::handlers::create),
        "getRequest" => |methods, filter| methods.on(filter, requests::handlers::get),
        "precheckRequest" => |methods, filter| methods.on(filter, requests::handlers::precheck),
        "transitionRequest" => |methods, filter| methods.on(filter, requests::transitions::transition),
        "saveVerification" => |methods, filter| methods.on(filter, requests::actions::save_verification),
        "sendPreviewFeedback" => |methods, filter| methods.on(filter, requests::actions::preview_feedback),
        "updateRegression" => |methods, filter| methods.on(filter, requests::actions::update_regression),
        "updateRelease" => |methods, filter| methods.on(filter, requests::actions::update_release),
        "regressionQueue" => |methods, filter| methods.on(filter, requests::actions::regression_queue),
        "listDrafts" => |methods, filter| methods.on(filter, drafts::handlers::list),
        "createDraft" => |methods, filter| methods.on(filter, drafts::handlers::create),
        "getDraft" => |methods, filter| methods.on(filter, drafts::handlers::get),
        "updateDraft" => |methods, filter| methods.on(filter, drafts::handlers::update),
        "deleteDraft" => |methods, filter| methods.on(filter, drafts::handlers::delete),
        "listNotifications" => |methods, filter| methods.on(filter, notifications::handlers::list),
        "markNotificationRead" => |methods, filter| methods.on(filter, notifications::handlers::mark_read),
        "markAllNotificationsRead" => |methods, filter| methods.on(filter, notifications::handlers::mark_all_read),
        "getNotificationSettings" => |methods, filter| methods.on(filter, notifications::handlers::get_settings),
        "updateNotificationSettings" => |methods, filter| methods.on(filter, notifications::handlers::update_settings),
        "uploadAttachment" => |methods, filter| {
            methods.on(filter, attachments::handlers::upload).layer(DefaultBodyLimit::max(attachments::BODY_LIMIT))
        },
        "dashboard" => |methods, filter| methods.on(filter, dashboard::summary),
        "releases" => |methods, filter| methods.on(filter, dashboard::releases),
        "getBudget" => |methods, filter| methods.on(filter, ai::handlers::get_budget),
        "updateBudget" => |methods, filter| methods.on(filter, ai::handlers::update_budget),
        "getAiSettings" => |methods, filter| methods.on(filter, ai::handlers::get_settings),
        "updateAiSettings" => |methods, filter| methods.on(filter, ai::handlers::update_settings),
        "setProviderKey" => |methods, filter| methods.on(filter, ai::handlers::set_provider_key),
        "removeProviderKey" => |methods, filter| methods.on(filter, ai::handlers::remove_provider_key),
        "getAiPricing" => |methods, filter| methods.on(filter, ai::handlers::get_pricing),
        "updateAiPricing" => |methods, filter| methods.on(filter, ai::handlers::update_pricing),
        "aiUsage" => |methods, filter| methods.on(filter, ai::handlers::report),
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
