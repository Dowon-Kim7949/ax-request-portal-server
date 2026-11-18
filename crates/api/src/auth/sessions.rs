//! The `sessions` table: one row per sign-in, found by the token's SHA-256.

use std::time::Duration;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::token;
use crate::config::SessionPolicy;
use crate::users::User;

/// The signed-in person behind a request's bearer token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentUser {
    pub user: User,
    pub session_id: Uuid,
}

/// Starts a session for `user_id` and returns its bearer token. Ended sessions
/// are cleared out on the way.
///
/// # Errors
///
/// Database failures.
pub async fn start(db: &PgPool, policy: &SessionPolicy, user_id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query("delete from sessions where expires_at <= now()").execute(db).await?;
    let token = token::generate();
    sqlx::query(
        "insert into sessions (user_id, token_hash, expires_at, absolute_expires_at)
         values ($1, $2, now() + make_interval(secs => $3), now() + make_interval(secs => $4))",
    )
    .bind(user_id)
    .bind(token::digest(&token).as_slice())
    .bind(policy.idle.as_secs_f64())
    .bind(policy.max.as_secs_f64())
    .execute(db)
    .await?;
    Ok(token)
}

/// The active person behind `token`, or `None` when the token is unknown, its
/// session has ended, or the account has been deactivated since.
///
/// Using a session moves its end `policy.idle` ahead (never past its absolute
/// end). The write happens at most about once a minute per session.
///
/// # Errors
///
/// Database failures.
pub async fn authenticate(
    db: &PgPool,
    policy: &SessionPolicy,
    token: &str,
) -> Result<Option<CurrentUser>, sqlx::Error> {
    if !token.starts_with(token::PREFIX) {
        return Ok(None);
    }
    let row = sqlx::query(
        "select s.id as session_id,
                s.last_seen_at < now() - make_interval(secs => $2) as stale,
                u.id, u.name, u.email, u.role, u.team_id, u.active
           from sessions s
           join users u on u.id = s.user_id
          where s.token_hash = $1 and s.expires_at > now() and u.active",
    )
    .bind(token::digest(token).as_slice())
    .bind(touch_interval(policy).as_secs_f64())
    .fetch_optional(db)
    .await?;
    let Some(row) = row else { return Ok(None) };

    let session_id: Uuid = row.try_get("session_id")?;
    if row.try_get::<bool, _>("stale")? {
        sqlx::query(
            "update sessions
                set last_seen_at = now(),
                    expires_at = least(now() + make_interval(secs => $2), absolute_expires_at)
              where id = $1",
        )
        .bind(session_id)
        .bind(policy.idle.as_secs_f64())
        .execute(db)
        .await?;
    }
    Ok(Some(CurrentUser { user: User::from_row(&row)?, session_id }))
}

/// Ends one session (sign-out).
///
/// # Errors
///
/// Database failures.
pub async fn end(db: &PgPool, session_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("delete from sessions where id = $1").bind(session_id).execute(db).await?;
    Ok(())
}

/// Ends every session of a person, e.g. after a password reset.
///
/// # Errors
///
/// Database failures.
pub async fn end_all(db: &PgPool, user_id: Uuid) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("delete from sessions where user_id = $1").bind(user_id).execute(db).await?.rows_affected())
}

/// How stale `last_seen_at` may get before a request writes it again: a
/// minute, or less for very short idle limits.
fn touch_interval(policy: &SessionPolicy) -> Duration {
    (policy.idle / 10).min(Duration::from_secs(60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_at_most_once_a_minute() {
        let policy = SessionPolicy::default();
        assert_eq!(touch_interval(&policy), Duration::from_secs(60));
        let short = SessionPolicy { idle: Duration::from_secs(100), ..policy };
        assert_eq!(touch_interval(&short), Duration::from_secs(10));
    }
}
