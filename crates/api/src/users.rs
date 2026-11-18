//! People, as the contract's `User` shows them.

use serde::Serialize;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::roles::Role;

/// The contract's `User`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub role: Role,
    /// Left out of the JSON when the person has no team: the contract's
    /// `teamId` is an optional string, and the frontend rejects `null`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<Uuid>,
    pub active: bool,
}

impl User {
    /// Reads the `id, name, email, role, team_id, active` columns of a row.
    ///
    /// # Errors
    ///
    /// Fails when a column is missing or the role is not one of the five.
    pub fn from_row(row: &PgRow) -> Result<Self, sqlx::Error> {
        let role: String = row.try_get("role")?;
        let role = Role::parse(&role)
            .ok_or_else(|| sqlx::Error::Decode(format!("users.role has an unknown value {role:?}").into()))?;
        Ok(Self {
            id: row.try_get("id")?,
            name: row.try_get("name")?,
            email: row.try_get("email")?,
            role,
            team_id: row.try_get("team_id")?,
            active: row.try_get("active")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serialises_as_the_contracts_user() {
        let id = Uuid::nil();
        let mut user = User {
            id,
            name: "Ada".to_owned(),
            email: "ada@example.com".to_owned(),
            role: Role::Admin,
            team_id: None,
            active: true,
        };
        assert_eq!(
            serde_json::to_value(&user).unwrap(),
            json!({ "id": id.to_string(), "name": "Ada", "email": "ada@example.com", "role": "admin", "active": true })
        );
        user.team_id = Some(id);
        assert_eq!(serde_json::to_value(&user).unwrap()["teamId"], id.to_string());
    }
}
