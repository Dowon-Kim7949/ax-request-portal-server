//! State shared by every handler.

use sqlx::PgPool;

use crate::ai::claude::ClaudeProvider;
use crate::attachments::Files;

use crate::config::{AiSettings, BusinessTimeZone, DraftPolicy, LoginPolicy, SessionPolicy, SetupLinkPolicy};
use crate::error::ApiError;
use crate::lang::Lang;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    /// `None` when `DATABASE_URL` is unset (local runs and tests).
    pub db: Option<PgPool>,
    pub sessions: SessionPolicy,
    pub setup_links: SetupLinkPolicy,
    pub login: LoginPolicy,
    pub time_zone: BusinessTimeZone,
    pub ai: AiSettings,
    pub drafts: DraftPolicy,
    /// Uploaded attachments and their download links.
    pub files: Files,
    /// The Claude provider when `AX_AI_PROVIDER=anthropic`: the precheck
    /// asks it directly; the report worker holds its own.
    pub claude: Option<ClaudeProvider>,
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
