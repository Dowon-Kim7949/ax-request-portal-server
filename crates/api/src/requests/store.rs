//! Requests in the database: the contract's `WorkRequest` assembled from its
//! columns, its JSONB `details` and its timeline (`request_events`).

use serde_json::{Map, Value, json};
use sqlx::postgres::PgRow;
use sqlx::{PgExecutor, Postgres, Row, Transaction};
use uuid::Uuid;

use super::lifecycle::Status;
use crate::roles::Permission;
use crate::users::User;

/// `column` as RFC 3339 in UTC with milliseconds and a `Z`, as JavaScript's
/// `toISOString` writes it.
pub fn iso(column: &str) -> String {
    format!(r#"to_char({column} at time zone 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.MS"Z"')"#)
}

fn select() -> String {
    format!(
        "select r.id, r.project_id, r.requester_id, r.assignee_id, r.status, r.type, r.title,
                to_char(r.desired_date, 'YYYY-MM-DD') as desired_date,
                {} as created_at, {} as updated_at, r.details
           from requests r",
        iso("r.created_at"),
        iso("r.updated_at")
    )
}

/// A field of a JSON object, or `null` when it is missing (indexing a
/// `serde_json::Map` directly would panic).
pub fn field<'a>(map: &'a Map<String, Value>, key: &str) -> &'a Value {
    static NULL: Value = Value::Null;
    map.get(key).unwrap_or(&NULL)
}

/// Who did something on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    User(Uuid),
    Ai,
    System,
}

/// A request as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredRequest {
    pub id: i64,
    pub project_id: Uuid,
    pub requester_id: Uuid,
    pub assignee_id: Option<Uuid>,
    pub status: Status,
    /// `type`
    pub kind: String,
    pub title: String,
    pub desired_date: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Every other field of the contract's `WorkRequest`, in its shape.
    pub details: Map<String, Value>,
    /// `TimelineEntry` objects, oldest first.
    pub timeline: Vec<Value>,
}

impl StoredRequest {
    fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let status: String = row.try_get("status")?;
        let status = Status::parse(&status)
            .ok_or_else(|| sqlx::Error::Decode(format!("requests.status has an unknown value {status:?}").into()))?;
        let details: Value = row.try_get("details")?;
        Ok(Self {
            id: row.try_get("id")?,
            project_id: row.try_get("project_id")?,
            requester_id: row.try_get("requester_id")?,
            assignee_id: row.try_get("assignee_id")?,
            status,
            kind: row.try_get("type")?,
            title: row.try_get("title")?,
            desired_date: row.try_get("desired_date")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            details: match details {
                Value::Object(map) => map,
                _ => Map::new(),
            },
            timeline: Vec::new(),
        })
    }

    /// The request with this id and its timeline, or `None`.
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn find(db: &mut sqlx::PgConnection, id: i64) -> Result<Option<Self>, sqlx::Error> {
        let row = sqlx::query(&format!("{} where r.id = $1", select())).bind(id).fetch_optional(&mut *db).await?;
        let Some(row) = row else { return Ok(None) };
        let mut request = Self::from_row(&row)?;
        request.timeline = timeline(&mut *db, id).await?;
        Ok(Some(request))
    }

    /// Like [`StoredRequest::find`], locking the row until the transaction
    /// ends, so changes to one request apply one after another.
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn find_for_update(tx: &mut Transaction<'_, Postgres>, id: i64) -> Result<Option<Self>, sqlx::Error> {
        let row =
            sqlx::query(&format!("{} where r.id = $1 for update", select())).bind(id).fetch_optional(&mut **tx).await?;
        let Some(row) = row else { return Ok(None) };
        let mut request = Self::from_row(&row)?;
        request.timeline = timeline(&mut **tx, id).await?;
        Ok(Some(request))
    }

    /// Writes status, assignee and details back; `updated_at` is the
    /// transaction's time. The timeline is appended separately ([`add_event`]).
    ///
    /// # Errors
    ///
    /// Database failures.
    pub async fn save(&mut self, tx: &mut Transaction<'_, Postgres>) -> Result<(), sqlx::Error> {
        self.updated_at = sqlx::query_scalar(&format!(
            "update requests set status = $2, assignee_id = $3, details = $4, updated_at = now()
              where id = $1
              returning {}",
            iso("updated_at")
        ))
        .bind(self.id)
        .bind(self.status.as_str())
        .bind(self.assignee_id)
        .bind(Value::Object(self.details.clone()))
        .fetch_one(&mut **tx)
        .await?;
        self.timeline = timeline(&mut **tx, self.id).await?;
        Ok(())
    }

    /// The number of timeline entries for `event`.
    pub fn count_events(&self, event: &str) -> usize {
        self.timeline.iter().filter(|e| e["event"] == event).count()
    }

    /// The developers responsible for it: the assignee, then the project's owners.
    pub fn responsible(&self, owners: &[Uuid]) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.assignee_id.into_iter().collect();
        ids.extend(owners.iter().filter(|o| Some(**o) != self.assignee_id));
        ids
    }

    /// Whether `user` may open it: they filed it, may read every request, or
    /// are responsible for it. Anyone else gets the same `404` as for a
    /// request that does not exist.
    pub fn can_read(&self, user: &User, owners: &[Uuid]) -> bool {
        self.requester_id == user.id
            || user.role.can(Permission::RequestReadAll)
            || self.responsible(owners).contains(&user.id)
    }

    /// The contract's `WorkRequest`.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("id".into(), json!(self.id));
        out.insert("title".into(), json!(self.title));
        out.insert("projectId".into(), json!(self.project_id));
        out.insert("type".into(), json!(self.kind));
        for (key, value) in &self.details {
            out.insert(key.clone(), value.clone());
        }
        if let Some(date) = &self.desired_date {
            out.insert("desiredDate".into(), json!(date));
        }
        out.insert("requesterId".into(), json!(self.requester_id));
        if let Some(assignee) = self.assignee_id {
            out.insert("assigneeId".into(), json!(assignee));
        }
        out.insert("status".into(), json!(self.status));
        out.insert("createdAt".into(), json!(self.created_at));
        out.insert("updatedAt".into(), json!(self.updated_at));
        out.insert("timeline".into(), Value::Array(self.timeline.clone()));
        Value::Object(out)
    }
}

/// The owners of a project, in order.
///
/// # Errors
///
/// Database failures.
pub async fn owners_of(db: impl PgExecutor<'_>, project: Uuid) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar("select user_id from project_owners where project_id = $1 order by position")
        .bind(project)
        .fetch_all(db)
        .await
}

/// The timeline of a request, oldest first, as `TimelineEntry` objects.
async fn timeline(db: impl PgExecutor<'_>, request: i64) -> Result<Vec<Value>, sqlx::Error> {
    let rows = sqlx::query(&format!(
        "select id, {} as at, actor_type, actor_id, event, from_status, to_status, note
           from request_events where request_id = $1 order by seq",
        iso("at")
    ))
    .bind(request)
    .fetch_all(db)
    .await?;
    rows.iter()
        .map(|row| {
            let actor_type: String = row.try_get("actor_type")?;
            let actor = match row.try_get::<Option<Uuid>, _>("actor_id")? {
                Some(user) => json!({ "type": "user", "userId": user }),
                None => json!({ "type": actor_type }),
            };
            let mut entry = Map::new();
            entry.insert("id".into(), json!(row.try_get::<Uuid, _>("id")?));
            entry.insert("at".into(), json!(row.try_get::<String, _>("at")?));
            entry.insert("actor".into(), actor);
            entry.insert("event".into(), json!(row.try_get::<String, _>("event")?));
            for (key, column) in [("from", "from_status"), ("to", "to_status"), ("note", "note")] {
                if let Some(value) = row.try_get::<Option<String>, _>(column)? {
                    entry.insert(key.into(), json!(value));
                }
            }
            Ok(Value::Object(entry))
        })
        .collect()
}

/// Appends a timeline entry, dated with the transaction's time.
///
/// # Errors
///
/// Database failures.
pub async fn add_event(
    tx: &mut Transaction<'_, Postgres>,
    request: i64,
    actor: Actor,
    event: &str,
    from: Option<Status>,
    to: Option<Status>,
    note: Option<&str>,
) -> Result<(), sqlx::Error> {
    let (actor_type, actor_id) = match actor {
        Actor::User(id) => ("user", Some(id)),
        Actor::Ai => ("ai", None),
        Actor::System => ("system", None),
    };
    sqlx::query(
        "insert into request_events (request_id, actor_type, actor_id, event, from_status, to_status, note)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(request)
    .bind(actor_type)
    .bind(actor_id)
    .bind(event)
    .bind(from.map(Status::as_str))
    .bind(to.map(Status::as_str))
    .bind(note)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Today on the business calendar, `YYYY-MM-DD`.
///
/// # Errors
///
/// Database failures, or a time zone PostgreSQL does not know.
pub async fn today(db: impl PgExecutor<'_>, time_zone: &str) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("select to_char((now() at time zone $1)::date, 'YYYY-MM-DD')")
        .bind(time_zone)
        .fetch_one(db)
        .await
}

/// The transaction's time as `toISOString` writes it: what changes made in it
/// are dated with.
///
/// # Errors
///
/// Database failures.
pub async fn now_iso(db: impl PgExecutor<'_>) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(&format!("select {}", iso("now()"))).fetch_one(db).await
}
