//! The database schema: migrations embedded in the binary.
//!
//! The server applies pending migrations when it starts; `ax-api migrate` does
//! the same and exits. sqlx takes an advisory lock, so several instances
//! starting at once apply each migration exactly once.

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

/// `crates/api/migrations`.
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies every pending migration.
///
/// # Errors
///
/// Fails when the database is unreachable or a migration does not apply.
pub async fn migrate(db: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(db).await
}
