//! Sign-in throttling: at most `AX_LOGIN_ATTEMPTS` failed sign-ins per email
//! within `AX_LOGIN_LOCKOUT`; the next attempt, even with the right password,
//! is refused until the lockout has passed.
//!
//! The attempt is counted before the password is checked (one atomic upsert),
//! so parallel guesses cannot all slip in before the count catches up, and a
//! successful sign-in clears it. Unknown emails are counted the same way, so
//! the answer never tells which emails exist. A lockout lets someone keep a
//! known email locked by guessing; that is the price of stopping the guessing,
//! and an admin can still reset the password with a setup link meanwhile.

use sqlx::PgPool;

use crate::config::LoginPolicy;

/// The row key: the email as typed, in lower case (emails match in any case).
fn key(email: &str) -> String {
    email.to_lowercase()
}

/// Counts an attempt. Returns `Some(seconds)` until the lockout ends when
/// this attempt is refused, `None` when the password may be checked.
///
/// # Errors
///
/// Database failures.
pub async fn attempt(db: &PgPool, policy: &LoginPolicy, email: &str) -> Result<Option<u64>, sqlx::Error> {
    let window = format!("{} seconds", policy.lockout.as_secs());
    // A row starts over once its lock has passed or, unlocked, once its
    // window has; such rows are no longer needed.
    sqlx::query(
        "delete from login_attempts
          where coalesce(locked_until <= now(), first_at < now() - $1::interval)",
    )
    .bind(&window)
    .execute(db)
    .await?;
    let (attempts, left): (i32, Option<f64>) = sqlx::query_as(
        "insert into login_attempts as a (email_key, attempts) values ($1, 1)
         on conflict (email_key) do update set
           attempts = case when coalesce(a.locked_until <= now(), a.first_at < now() - $2::interval)
                           then 1 else a.attempts + 1 end,
           first_at = case when coalesce(a.locked_until <= now(), a.first_at < now() - $2::interval)
                           then now() else a.first_at end,
           locked_until = case when coalesce(a.locked_until <= now(), a.first_at < now() - $2::interval) then null
                               when a.locked_until is null and a.attempts + 1 > $3 then now() + $2::interval
                               else a.locked_until end
         returning attempts, extract(epoch from locked_until - now())::float8",
    )
    .bind(key(email))
    .bind(&window)
    .bind(i32::try_from(policy.attempts).unwrap_or(i32::MAX))
    .fetch_one(db)
    .await?;
    if u32::try_from(attempts).unwrap_or(u32::MAX) <= policy.attempts {
        return Ok(None);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // a positive number of seconds
    Ok(Some(left.unwrap_or(0.0).ceil().max(1.0) as u64))
}

/// Forgets the attempts after a successful sign-in.
///
/// # Errors
///
/// Database failures.
pub async fn clear(db: &PgPool, email: &str) -> Result<(), sqlx::Error> {
    sqlx::query("delete from login_attempts where email_key = $1").bind(key(email)).execute(db).await.map(|_| ())
}
