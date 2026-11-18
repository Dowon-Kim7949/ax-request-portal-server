//! HTTP API server for the AX Request Portal.
//!
//! The server implements the contract in `contract/openapi.json`. Operations
//! that are not implemented yet are still routed: they answer `501` with the
//! contract's error body, so the frontend sees a clear "not implemented" instead
//! of a 404, and syncing a newer contract makes new operations show up at once.

pub mod accounts;
pub mod auth;
pub mod body;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod implemented;
pub mod lang;
pub mod roles;
pub mod routes;
pub mod state;
pub mod teams;
pub mod users;
pub mod validate;

pub use config::Config;
pub use routes::router;
pub use state::AppState;
