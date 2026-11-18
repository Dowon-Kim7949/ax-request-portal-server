//! The API contract this server implements.
//!
//! The contract is the frontend's `docs/openapi.json`, generated there from
//! `src/api/contract.ts`. This repository never edits it: `scripts/sync-contract.sh`
//! copies it byte for byte from a pinned frontend commit (recorded in
//! `contract/SOURCE`), and CI checks that the copy still matches that commit.

use std::sync::OnceLock;

use serde_json::Value;

/// The vendored OpenAPI document, byte for byte.
pub const OPENAPI_JSON: &str = include_str!("../../../contract/openapi.json");

/// Where the vendored document came from (`repo=` and `commit=` lines).
pub const SOURCE: &str = include_str!("../../../contract/SOURCE");

/// One operation of the contract: an HTTP method on a path template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Operation {
    /// The contract's `operationId`, e.g. `login`.
    pub id: String,
    /// Upper-case HTTP method, e.g. `POST`.
    pub method: String,
    /// Path template with `{param}` segments, e.g. `/requests/{id}`.
    pub path: String,
}

/// The parsed document.
///
/// # Panics
///
/// Panics if the vendored file is not valid JSON. A unit test guards this, so
/// a broken sync fails CI instead of the server at runtime.
pub fn document() -> &'static Value {
    static DOC: OnceLock<Value> = OnceLock::new();
    DOC.get_or_init(|| serde_json::from_str(OPENAPI_JSON).expect("contract/openapi.json is valid JSON"))
}

/// Every operation in the contract, in document order.
///
/// # Panics
///
/// Panics if the document has no `paths` object or an operation has no
/// `operationId`; both are covered by unit tests.
pub fn operations() -> &'static [Operation] {
    static OPS: OnceLock<Vec<Operation>> = OnceLock::new();
    OPS.get_or_init(|| {
        let paths = document()["paths"].as_object().expect("openapi.json has paths");
        let mut ops = Vec::new();
        for (path, item) in paths {
            let item = item.as_object().expect("a path item is an object");
            for (method, op) in item {
                if !matches!(method.as_str(), "get" | "post" | "put" | "patch" | "delete") {
                    continue;
                }
                let id = op["operationId"].as_str().expect("every operation has an operationId");
                ops.push(Operation { id: id.to_owned(), method: method.to_ascii_uppercase(), path: path.clone() });
            }
        }
        ops
    })
}

/// The error codes the contract allows in `ApiError.code`.
pub fn error_codes() -> Vec<&'static str> {
    document()["components"]["schemas"]["ApiError"]["properties"]["code"]["enum"]
        .as_array()
        .map(|codes| codes.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default()
}

/// The contract's own version (`info.version`).
pub fn version() -> &'static str {
    document()["info"]["version"].as_str().unwrap_or("unknown")
}

/// The frontend commit the contract was copied from.
pub fn source_commit() -> &'static str {
    SOURCE.lines().find_map(|line| line.strip_prefix("commit=")).map_or("unknown", str::trim)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_document_parses_and_has_operations() {
        assert!(document().is_object());
        assert!(!operations().is_empty());
    }

    #[test]
    fn operation_ids_are_unique() {
        let mut seen = HashSet::new();
        for op in operations() {
            assert!(seen.insert(op.id.as_str()), "duplicate operationId {}", op.id);
        }
    }

    #[test]
    fn paths_are_absolute_templates() {
        for op in operations() {
            assert!(op.path.starts_with('/'), "{} has a relative path {}", op.id, op.path);
            assert!(!op.path.ends_with('/') || op.path == "/", "{} ends with a slash", op.id);
        }
    }

    #[test]
    fn the_error_codes_are_listed() {
        let codes = error_codes();
        assert!(codes.contains(&"not_found"));
        assert!(codes.contains(&"internal"));
    }

    #[test]
    fn the_source_names_a_full_commit() {
        let commit = source_commit();
        assert_eq!(commit.len(), 40, "contract/SOURCE must pin a full commit SHA, got {commit}");
        assert!(commit.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
