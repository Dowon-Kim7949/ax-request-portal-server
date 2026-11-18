//! Teams, as the contract's `Team` shows them, and `listTeams`, `createTeam`, `updateTeam`.
//!
//! Records refer to a team by id, so renaming carries its history along.
//! Teams are never deleted: past AI usage still points at them.

pub mod handlers;

use serde::Serialize;
use uuid::Uuid;

/// The longest team name, in UTF-16 units after trimming (the frontend's `TEAM_NAME_MAX`).
pub const NAME_MAX: usize = 40;

/// The contract's `Team`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Team {
    pub id: Uuid,
    pub name: String,
}

impl Team {
    /// The team with this id, or `None` (also for an id that is not a UUID).
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn find(db: &sqlx::PgPool, id: &str) -> Result<Option<Self>, sqlx::Error> {
        let Ok(id) = Uuid::parse_str(id) else { return Ok(None) };
        sqlx::query_as("select id, name from teams where id = $1").bind(id).fetch_optional(db).await
    }
}
