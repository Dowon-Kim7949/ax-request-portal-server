//! `login`, `me` and `logout`.

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Serialize;
use serde_json::Value;
use sqlx::Row;

use super::{CurrentUser, password, sessions};
use crate::error::{ApiError, Issue};
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
/// tell which emails exist. The contract lists only `200` and `400` here; the
/// `401` follows the frontend's mock API, which the client is built against.
pub async fn login(State(state): State<AppState>, lang: Lang, body: Bytes) -> Result<Json<Session>, ApiError> {
    let (email, password) = login_body(&body, lang)?;
    let db = state.db(lang)?;

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
fn login_body(body: &[u8], lang: Lang) -> Result<(String, String), ApiError> {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        let message = lang.pick("JSON 객체를 보내 주세요.", "Send a JSON object.");
        return Err(ApiError::check_input(lang, vec![Issue { path: String::new(), message: message.to_owned() }]));
    };
    let email = value.get("email").and_then(Value::as_str).filter(|email| is_email(email));
    let password = value.get("password").and_then(Value::as_str).filter(|password| !password.is_empty());
    match (email, password) {
        (Some(email), Some(password)) => Ok((email.to_owned(), password.to_owned())),
        (email, password) => {
            let mut issues = Vec::new();
            if email.is_none() {
                let message = lang.pick("올바른 이메일 주소를 입력해 주세요.", "Enter a valid email address.");
                issues.push(Issue { path: "email".to_owned(), message: message.to_owned() });
            }
            if password.is_none() {
                let message = lang.pick("비밀번호를 입력해 주세요.", "Enter your password.");
                issues.push(Issue { path: "password".to_owned(), message: message.to_owned() });
            }
            Err(ApiError::check_input(lang, issues))
        }
    }
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
        assert_eq!(issue_paths("[]"), ["email", "password"]);
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
