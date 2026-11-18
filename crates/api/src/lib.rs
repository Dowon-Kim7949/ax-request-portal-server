//! HTTP API server for the AX Request Portal.
//!
//! The server implements the contract in `contract/openapi.json`. Operations
//! that are not implemented yet are still routed: they answer `501` with the
//! contract's error body, so the frontend sees a clear "not implemented" instead
//! of a 404, and syncing a newer contract makes new operations show up at once.

pub mod config;
pub mod error;
pub mod implemented;
pub mod lang;
pub mod routes;
pub mod state;

pub use config::Config;
pub use routes::router;
pub use state::AppState;
