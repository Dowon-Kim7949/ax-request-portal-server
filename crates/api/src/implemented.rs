//! Which contract operations have a real handler.
//!
//! Every other operation in `contract/openapi.json` is routed to a `501`
//! answer (see `routes`). When an operation gets a handler, add its
//! `operationId` here and an arm for its handler in `routes::implemented`.
//! Building the router panics if the two disagree, and a test checks that every
//! entry is in the contract.

/// `operationId`s with a real handler, in contract order.
pub const IMPLEMENTED: &[&str] =
    &["login", "logout", "me", "listUsers", "createUser", "updateUser", "listTeams", "createTeam", "updateTeam"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_in_contract_order() {
        let order: Vec<&str> =
            ax_contract::operations().iter().map(|op| op.id.as_str()).filter(|id| IMPLEMENTED.contains(id)).collect();
        assert_eq!(order, IMPLEMENTED);
    }

    #[test]
    fn every_implemented_operation_is_in_the_contract() {
        for id in IMPLEMENTED {
            assert!(
                ax_contract::operations().iter().any(|op| op.id == *id),
                "{id} is marked implemented but is not in contract/openapi.json"
            );
        }
    }
}
