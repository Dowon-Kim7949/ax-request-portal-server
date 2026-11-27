//! Projects, as the contract's `Project` shows them, and `listProjects`,
//! `createProject`, `updateProject`, `listRepositoryBranches`.
//!
//! Owners are kept in order in `project_owners` (a new request goes to the
//! first one); change targets and the rules to keep are JSONB lists in the
//! contract's shape, with ids the server gives and never changes. The
//! repository may name the branch the AI works from (`repository_branch`).

pub mod branches;
pub mod handlers;

use serde::Serialize;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{PgExecutor, Row};
use uuid::Uuid;

/// The contract's `Project`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub icon: String,
    pub repository: Repository,
    pub wiki_path: String,
    pub wiki_stats: WikiStats,
    pub owner_ids: Vec<Uuid>,
    /// `[{ id, name, wikiDoc? }]`
    pub targets: Value,
    /// `[{ id, text, reference? }]` (the `rules` column)
    pub constraints: Value,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Repository {
    pub provider: String,
    pub url: String,
    /// The branch the AI works from; `None` = the repository's default branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiStats {
    pub verified_docs: i32,
    pub total_docs: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<String>,
}

/// Columns of a project row, owners in order; `wiki_synced_at` is RFC 3339 in UTC.
pub const SELECT: &str = r#"
    select p.id, p.name, p.description, p.icon, p.repository_provider, p.repository_url,
           p.repository_branch, p.wiki_path,
           p.wiki_verified_docs, p.wiki_total_docs,
           to_char(p.wiki_synced_at at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"') as wiki_synced_at,
           p.targets, p.rules, p.active,
           coalesce((select array_agg(o.user_id order by o.position) from project_owners o where o.project_id = p.id),
                    '{}') as owner_ids
      from projects p"#;

impl Project {
    /// Reads a row of [`SELECT`].
    ///
    /// # Errors
    ///
    /// A missing or mistyped column.
    pub fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            description: row.try_get("description")?,
            icon: row.try_get("icon")?,
            repository: Repository {
                provider: row.try_get("repository_provider")?,
                url: row.try_get("repository_url")?,
                branch: row.try_get("repository_branch")?,
            },
            wiki_path: row.try_get("wiki_path")?,
            wiki_stats: WikiStats {
                verified_docs: row.try_get("wiki_verified_docs")?,
                total_docs: row.try_get("wiki_total_docs")?,
                last_synced_at: row.try_get("wiki_synced_at")?,
            },
            owner_ids: row.try_get("owner_ids")?,
            targets: row.try_get("targets")?,
            constraints: row.try_get("rules")?,
            active: row.try_get("active")?,
        })
    }

    /// Every project, oldest first.
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn all(db: impl PgExecutor<'_>) -> Result<Vec<Self>, sqlx::Error> {
        let rows = sqlx::query(&format!("{SELECT} order by p.created_at, p.id")).fetch_all(db).await?;
        rows.iter().map(Self::from_row).collect()
    }

    /// The project with this id, or `None` (also for an id that is not a UUID).
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn find(db: impl PgExecutor<'_>, id: &str) -> Result<Option<Self>, sqlx::Error> {
        let Ok(id) = Uuid::parse_str(id) else { return Ok(None) };
        let row = sqlx::query(&format!("{SELECT} where p.id = $1")).bind(id).fetch_optional(db).await?;
        row.as_ref().map(Self::from_row).transpose()
    }

    /// The ids of the project's change targets.
    pub fn target_ids(&self) -> impl Iterator<Item = &str> {
        self.targets.as_array().into_iter().flatten().filter_map(|t| t["id"].as_str())
    }
}
