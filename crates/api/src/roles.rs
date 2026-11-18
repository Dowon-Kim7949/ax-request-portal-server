//! Roles and permissions, ported from the frontend's `src/domain/roles.ts`.
//!
//! The frontend hides what a role may not do; the server is what enforces it,
//! on every operation. The default table here equals the frontend's
//! `DEFAULT_ROLE_PERMISSIONS`. A deployment that overrides `rolePermissions` in
//! the frontend's `app.config.ts` must change this table to match.

use serde::{Deserialize, Serialize};

/// The contract's `Role`. A unit test keeps the names equal to the contract's enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Planning or sales staff who file requests.
    Requester,
    /// A requester who also sees the company-wide dashboard.
    Executive,
    /// Reviews reports, runs AI work, approves merges.
    Developer,
    /// Manages projects and people of the organisation.
    Admin,
    /// Runs the service: AI providers and models, prices, AI budget and usage.
    /// Provisioned by the service, never by an admin.
    Operator,
}

impl Role {
    pub const ALL: [Self; 5] = [Self::Requester, Self::Executive, Self::Developer, Self::Admin, Self::Operator];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requester => "requester",
            Self::Executive => "executive",
            Self::Developer => "developer",
            Self::Admin => "admin",
            Self::Operator => "operator",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.as_str() == value)
    }

    /// What the role may do (`DEFAULT_ROLE_PERMISSIONS`).
    pub const fn permissions(self) -> &'static [Permission] {
        use Permission::*;
        match self {
            Self::Requester => &[RequestCreate, RequestReadOwn, ScheduleView],
            Self::Executive => &[RequestCreate, RequestReadOwn, ScheduleView, RequestReadAll, DashboardView],
            Self::Developer => &[
                RequestCreate,
                RequestReadOwn,
                RequestReadAll,
                RequestReview,
                ReportRead,
                RegressionView,
                ScheduleView,
            ],
            // Everything but the operator's permissions.
            Self::Admin => &[
                RequestCreate,
                RequestReadOwn,
                RequestReadAll,
                RequestReview,
                ReportRead,
                RegressionView,
                ScheduleView,
                DashboardView,
                ProjectManage,
                UserManage,
            ],
            Self::Operator => &OPERATOR_PERMISSIONS,
        }
    }

    pub fn can(self, permission: Permission) -> bool {
        self.permissions().contains(&permission)
    }

    /// Whether an admin may give someone this role, or change someone who has it.
    /// Operator accounts are provisioned by the service (`ax-api user create`),
    /// so the API answers `403` with `operatorProvisioned` otherwise.
    pub const fn admin_may_assign(self) -> bool {
        !matches!(self, Self::Operator)
    }
}

/// The frontend's `PERMISSIONS`, in the same order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// File a new request.
    RequestCreate,
    /// See requests I filed.
    RequestReadOwn,
    /// See every request (dashboards, developer queues).
    RequestReadAll,
    /// Developer actions: request info, reject, run AI work, review the merge request, release.
    RequestReview,
    /// The full feasibility report. Without it only the plain-language summary is shown.
    ReportRead,
    RegressionView,
    ScheduleView,
    DashboardView,
    ProjectManage,
    UserManage,
    /// AI budget: monthly and per-request limits, alert threshold.
    BudgetManage,
    /// AI providers and keys, model per task and project, price table.
    AgentManage,
    /// AI usage and cost by provider, model, task and request.
    UsageView,
}

/// What the service operator holds; an organisation admin does not.
pub const OPERATOR_PERMISSIONS: [Permission; 3] =
    [Permission::BudgetManage, Permission::AgentManage, Permission::UsageView];

impl Permission {
    pub const ALL: [Self; 13] = [
        Self::RequestCreate,
        Self::RequestReadOwn,
        Self::RequestReadAll,
        Self::RequestReview,
        Self::ReportRead,
        Self::RegressionView,
        Self::ScheduleView,
        Self::DashboardView,
        Self::ProjectManage,
        Self::UserManage,
        Self::BudgetManage,
        Self::AgentManage,
        Self::UsageView,
    ];

    /// The frontend's name, e.g. `request:read:own`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RequestCreate => "request:create",
            Self::RequestReadOwn => "request:read:own",
            Self::RequestReadAll => "request:read:all",
            Self::RequestReview => "request:review",
            Self::ReportRead => "report:read",
            Self::RegressionView => "regression:view",
            Self::ScheduleView => "schedule:view",
            Self::DashboardView => "dashboard:view",
            Self::ProjectManage => "project:manage",
            Self::UserManage => "user:manage",
            Self::BudgetManage => "budget:manage",
            Self::AgentManage => "agent:manage",
            Self::UsageView => "usage:view",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn names(role: Role) -> Vec<&'static str> {
        role.permissions().iter().map(|p| p.as_str()).collect()
    }

    #[test]
    fn roles_match_the_contract() {
        let ours: BTreeSet<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
        let contract: BTreeSet<&str> = ax_contract::schema_enum("Role").into_iter().collect();
        assert_eq!(ours, contract, "Role drifted from the contract's Role enum");
        for role in Role::ALL {
            assert_eq!(serde_json::to_value(role).unwrap(), role.as_str());
            assert_eq!(Role::parse(role.as_str()), Some(role));
        }
        assert_eq!(Role::parse("root"), None);
    }

    /// `DEFAULT_ROLE_PERMISSIONS` in the frontend's `src/domain/roles.ts`.
    #[test]
    fn the_default_table_matches_the_frontend() {
        assert_eq!(names(Role::Requester), ["request:create", "request:read:own", "schedule:view"]);
        assert_eq!(
            names(Role::Executive),
            ["request:create", "request:read:own", "schedule:view", "request:read:all", "dashboard:view"]
        );
        assert_eq!(
            names(Role::Developer),
            [
                "request:create",
                "request:read:own",
                "request:read:all",
                "request:review",
                "report:read",
                "regression:view",
                "schedule:view"
            ]
        );
        assert_eq!(names(Role::Operator), ["budget:manage", "agent:manage", "usage:view"]);
    }

    #[test]
    fn admin_holds_everything_but_the_operators_permissions() {
        let expected: Vec<Permission> =
            Permission::ALL.into_iter().filter(|p| !OPERATOR_PERMISSIONS.contains(p)).collect();
        assert_eq!(Role::Admin.permissions(), expected.as_slice());
        assert!(Role::Admin.can(Permission::UserManage));
        assert!(!Role::Admin.can(Permission::BudgetManage));
    }

    #[test]
    fn the_operator_holds_no_request_permissions() {
        for p in Permission::ALL {
            assert_eq!(Role::Operator.can(p), OPERATOR_PERMISSIONS.contains(&p), "{}", p.as_str());
        }
    }

    #[test]
    fn permission_names_are_unique() {
        let set: BTreeSet<&str> = Permission::ALL.iter().map(|p| p.as_str()).collect();
        assert_eq!(set.len(), 13);
    }

    #[test]
    fn only_the_service_provisions_operators() {
        assert!(!Role::Operator.admin_may_assign());
        for role in [Role::Requester, Role::Executive, Role::Developer, Role::Admin] {
            assert!(role.admin_may_assign(), "{}", role.as_str());
        }
    }
}
