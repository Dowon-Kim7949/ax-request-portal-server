//! `login`, `me` and `logout`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use sqlx::Row;

use super::{CurrentUser, password, sessions, throttle};
use crate::body::{Body, Text};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::state::AppState;
use crate::users::User;
use crate::validate::is_email;

/// The contract's `Session`.
#[derive(Debug, Serialize)]
pub struct Session {
    pub token: String,
    pub user: User,
}

/// `POST /auth/login`.
///
/// `400` (`checkInput`) when the body does not match `LoginBody`. Any other
/// failure (no such account, a deactivated one, no password set, a wrong
/// password) is the same `401` (`wrongCredentials`), so the answer does not
/// tell which emails exist.
///
/// Each attempt is counted first (`auth::throttle`): past `AX_LOGIN_ATTEMPTS`
/// within `AX_LOGIN_LOCKOUT` the answer is a `401` saying when to try again,
/// whatever the password, and for unknown emails alike. Signing in clears the
/// count.
pub async fn login(State(state): State<AppState>, lang: Lang, body: Bytes) -> Result<Json<Session>, ApiError> {
    let (email, password) = login_body(&body, lang)?;
    let db = state.db(lang)?;
    if let Some(seconds) =
        throttle::attempt(db, &state.login, &email).await.map_err(|error| ApiError::database(lang, &error))?
    {
        return Err(ApiError::sign_in_locked(lang, seconds));
    }

    let row = sqlx::query(
        "select u.id, u.name, u.email, u.role, u.team_id, u.active, u.password_hash
           from users u
          where lower(u.email) = lower($1)",
    )
    .bind(&email)
    .fetch_optional(db)
    .await
    .map_err(|error| ApiError::database(lang, &error))?;

    let (user, stored) = match &row {
        Some(row) => {
            let user = User::from_row(row).map_err(|error| ApiError::database(lang, &error))?;
            let stored: Option<String> =
                row.try_get("password_hash").map_err(|error| ApiError::database(lang, &error))?;
            (Some(user), stored)
        }
        None => (None, None),
    };

    let matches = password::check(password, stored).await;
    let Some(user) = user.filter(|user| matches && user.active) else {
        return Err(ApiError::wrong_credentials(lang));
    };

    throttle::clear(db, &email).await.map_err(|error| ApiError::database(lang, &error))?;
    let token =
        sessions::start(db, &state.sessions, user.id).await.map_err(|error| ApiError::database(lang, &error))?;
    Ok(Json(Session { token, user }))
}

/// `GET /auth/me`.
pub async fn me(current: CurrentUser) -> Json<User> {
    Json(current.user)
}

/// `POST /auth/logout`: ends the session the request's token belongs to.
/// The token answers `401` from then on.
pub async fn logout(State(state): State<AppState>, lang: Lang, current: CurrentUser) -> Result<StatusCode, ApiError> {
    let db = state.db(lang)?;
    sessions::end(db, current.session_id).await.map_err(|error| ApiError::database(lang, &error))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Checks the body against the contract's `LoginBody`. The email is compared
/// as typed (no trimming), like the frontend's validation.
fn login_body(bytes: &[u8], lang: Lang) -> Result<(String, String), ApiError> {
    const EMAIL: Text = ("올바른 이메일 주소를 입력해 주세요.", "Enter a valid email address.");
    const PASSWORD: Text = ("비밀번호를 입력해 주세요.", "Enter your password.");
    let mut body = Body::parse(bytes, lang)?;
    let email = body.required("email", EMAIL, |v| v.as_str().filter(|e| is_email(e)).map(str::to_owned));
    let password = body.required("password", PASSWORD, |v| v.as_str().filter(|p| !p.is_empty()).map(str::to_owned));
    body.finish()?;
    let (Some(email), Some(password)) = (email, password) else {
        unreachable!("finish() fails when a required field is missing")
    };
    Ok((email, password))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue_paths(body: &str) -> Vec<String> {
        login_body(body.as_bytes(), Lang::En).unwrap_err().body.issues.into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn checks_the_login_body() {
        assert_eq!(
            login_body(br#"{"email":"Ada@Example.com","password":"x"}"#, Lang::En).unwrap(),
            ("Ada@Example.com".to_owned(), "x".to_owned())
        );
        assert_eq!(issue_paths(r#"{"email":"ada@example.com"}"#), ["password"]);
        assert_eq!(issue_paths(r#"{"email":"ada@example.com","password":""}"#), ["password"]);
        assert_eq!(issue_paths(r#"{"email":" ada@example.com","password":"x"}"#), ["email"]);
        assert_eq!(issue_paths(r#"{"email":1,"password":2}"#), ["email", "password"]);
        assert_eq!(issue_paths("[]"), [""]);
        assert_eq!(issue_paths(""), [""]);
        assert_eq!(issue_paths("{"), [""]);
    }

    #[test]
    fn a_bad_body_is_check_input() {
        let error = login_body(b"{}", Lang::Ko).unwrap_err();
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.body.message, "입력값을 확인해 주세요.");
    }
}
