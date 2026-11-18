//! Account provisioning by the service itself (`ax-api user ...`).
//!
//! This is how the first admin is made, and the only way to make an operator:
//! operator accounts are provisioned by the service, never through the API.
//! There are no built-in or demo accounts.

use std::fmt;

use sqlx::PgPool;

use crate::auth::{password, sessions};
use crate::roles::Role;
use crate::users::User;
use crate::validate::is_email;

/// Passwords set here must be at least this long (in characters). Sign-in
/// itself accepts whatever was set, as the contract's `LoginBody` does.
pub const MIN_PASSWORD_CHARS: usize = 12;

#[derive(Debug)]
pub enum AccountError {
    InvalidEmail,
    EmptyName,
    ShortPassword,
    EmailTaken,
    NoSuchAccount,
    Database(sqlx::Error),
}

impl fmt::Display for AccountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEmail => f.write_str("not a valid email address"),
            Self::EmptyName => f.write_str("the name must not be empty"),
            Self::ShortPassword => write!(f, "the password must be at least {MIN_PASSWORD_CHARS} characters long"),
            Self::EmailTaken => f.write_str("an account with this email already exists"),
            Self::NoSuchAccount => f.write_str("no account has this email"),
            Self::Database(error) => write!(f, "database error: {error}"),
        }
    }
}

impl std::error::Error for AccountError {}

impl From<sqlx::Error> for AccountError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// What `ax-api user create` asks for.
#[derive(Debug, Clone)]
pub struct NewAccount {
    pub email: String,
    pub name: String,
    pub role: Role,
    pub password: String,
}

/// Creates an active account with a password and no team.
///
/// # Errors
///
/// [`AccountError::EmailTaken`] when another account has the email in any
/// case; validation errors for a bad email, an empty name or a short password.
pub async fn create(db: &PgPool, account: NewAccount) -> Result<User, AccountError> {
    let NewAccount { email, name, role, password } = account;
    if !is_email(&email) {
        return Err(AccountError::InvalidEmail);
    }
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err(AccountError::EmptyName);
    }
    let hash = hash_password(password).await?;

    let row = sqlx::query(
        "insert into users (email, name, role, password_hash)
         values ($1, $2, $3, $4)
         on conflict ((lower(email))) do nothing
         returning id, name, email, role, team_id, active",
    )
    .bind(&email)
    .bind(&name)
    .bind(role.as_str())
    .bind(hash)
    .fetch_optional(db)
    .await?;
    let row = row.ok_or(AccountError::EmailTaken)?;
    Ok(User::from_row(&row)?)
}

/// Sets or resets the password of the account with this email (in any case)
/// and signs that account out everywhere. Works for deactivated accounts too;
/// they still cannot sign in until reactivated.
///
/// # Errors
///
/// [`AccountError::NoSuchAccount`], or [`AccountError::ShortPassword`].
pub async fn set_password(db: &PgPool, email: &str, password: String) -> Result<User, AccountError> {
    let hash = hash_password(password).await?;
    let row = sqlx::query(
        "update users set password_hash = $2, updated_at = now()
          where lower(email) = lower($1)
          returning id, name, email, role, team_id, active",
    )
    .bind(email)
    .bind(hash)
    .fetch_optional(db)
    .await?;
    let user = User::from_row(&row.ok_or(AccountError::NoSuchAccount)?)?;
    sessions::end_all(db, user.id).await?;
    Ok(user)
}

async fn hash_password(password: String) -> Result<String, AccountError> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(AccountError::ShortPassword);
    }
    tokio::task::spawn_blocking(move || password::hash(&password))
        .await
        .map_err(|error| AccountError::Database(sqlx::Error::Protocol(format!("password hashing failed: {error}"))))
}
