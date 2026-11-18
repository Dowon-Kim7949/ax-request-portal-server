//! Sign-in: passwords, bearer tokens, sessions and the `login` / `me` /
//! `logout` operations.
//!
//! Every operation but `login` needs `Authorization: Bearer <token>`. A
//! missing, unknown or ended token, or a deactivated account, answers `401`
//! (`signInAgain`); a role without the permission answers `403` (`forbidden`).
//! Both match the frontend's mock API.

pub mod handlers;
pub mod password;
pub mod sessions;
pub mod token;

use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;

pub use self::sessions::CurrentUser;
use crate::error::ApiError;
use crate::lang::Lang;
use crate::roles::Permission;
use crate::state::AppState;
use crate::users::User;

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let lang = Lang::of(&parts.headers);
        let token = bearer_token(&parts.headers).ok_or_else(|| ApiError::sign_in_again(lang))?;
        let db = state.db(lang)?;
        sessions::authenticate(db, &state.sessions, token)
            .await
            .map_err(|error| ApiError::database(lang, &error))?
            .ok_or_else(|| ApiError::sign_in_again(lang))
    }
}

/// The token of an `Authorization: Bearer <token>` header.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

/// `403` unless the person's role has `permission`.
///
/// # Errors
///
/// [`ApiError::forbidden`].
pub fn require(user: &User, permission: Permission, lang: Lang) -> Result<(), ApiError> {
    if user.role.can(permission) { Ok(()) } else { Err(ApiError::forbidden(lang)) }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use uuid::Uuid;

    use super::*;
    use crate::error::ErrorCode;
    use crate::roles::Role;

    fn headers(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn reads_bearer_tokens() {
        assert_eq!(bearer_token(&headers("Bearer axs_abc")), Some("axs_abc"));
        assert_eq!(bearer_token(&headers("bearer axs_abc")), Some("axs_abc"));
        assert_eq!(bearer_token(&headers("Basic YWRhOnB3")), None);
        assert_eq!(bearer_token(&headers("Bearer ")), None);
        assert_eq!(bearer_token(&headers("Bearer")), None);
        assert_eq!(bearer_token(&HeaderMap::new()), None);
    }

    #[test]
    fn require_answers_403_without_the_permission() {
        let user = |role| User {
            id: Uuid::nil(),
            name: "A".to_owned(),
            email: "a@example.com".to_owned(),
            role,
            team_id: None,
            active: true,
        };
        assert!(require(&user(Role::Admin), Permission::UserManage, Lang::En).is_ok());
        let error = require(&user(Role::Admin), Permission::BudgetManage, Lang::En).unwrap_err();
        assert_eq!((error.status.as_u16(), error.body.code), (403, ErrorCode::Forbidden));
        assert_eq!(error.body.message, "You do not have permission to do this.");
        assert!(require(&user(Role::Operator), Permission::RequestCreate, Lang::Ko).is_err());
    }
}
