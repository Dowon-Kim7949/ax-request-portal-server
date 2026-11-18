//! State shared by every handler.

use sqlx::PgPool;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// `None` when `DATABASE_URL` is unset (local runs and tests).
    pub db: Option<PgPool>,
}
