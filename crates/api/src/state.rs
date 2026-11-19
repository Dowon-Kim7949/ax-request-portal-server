//! State shared by every handler.

use sqlx::PgPool;

use crate::config::{AiSettings, BusinessTimeZone, SessionPolicy, SetupLinkPolicy};
use crate::error::ApiError;
use crate::lang::Lang;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// `None` when `DATABASE_URL` is unset (local runs and tests).
    pub db: Option<PgPool>,
    pub sessions: SessionPolicy,
    pub setup_links: SetupLinkPolicy,
    pub time_zone: BusinessTimeZone,
    pub ai: AiSettings,
}

impl AppState {
    /// The pool, or `500` when the server runs without a database.
    ///
    /// # Errors
    ///
    /// [`ApiError::no_database`] when `DATABASE_URL` is unset.
    pub fn db(&self, lang: Lang) -> Result<&PgPool, ApiError> {
        self.db.as_ref().ok_or_else(|| ApiError::no_database(lang))
    }
}
