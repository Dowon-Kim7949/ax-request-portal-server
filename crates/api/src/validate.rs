//! Field checks taken from the contract, so the server accepts what the
//! frontend's own validation accepts.

use std::sync::OnceLock;

use regex::Regex;

/// The contract's email pattern (`LoginBody.email`, also used by `User` and `UserCreate`).
///
/// # Panics
///
/// Panics if the contract has no such pattern or it does not compile; a unit
/// test covers both, so a contract sync that breaks it fails CI.
pub fn is_email(value: &str) -> bool {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    EMAIL
        .get_or_init(|| {
            let pattern =
                ax_contract::property_pattern("LoginBody", "email").expect("the contract has LoginBody.email");
            Regex::new(pattern).expect("the contract's email pattern compiles")
        })
        .is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_what_the_frontend_accepts() {
        for ok in ["ada@example.com", "Ada.Lovelace+ax@mail.example.co", "o'neil@example.org"] {
            assert!(is_email(ok), "{ok}");
        }
        for bad in [
            "",
            "ada",
            "ada@",
            "@example.com",
            "ada@example",
            " ada@example.com",
            "ada@example.com ",
            "a..b@example.com",
        ] {
            assert!(!is_email(bad), "{bad:?}");
        }
    }
}
