//! Password hashing: Argon2id with the OWASP parameters (19 MiB, 2 passes, 1 lane).
//!
//! Hashes are stored as PHC strings, which carry their own parameters, so a
//! later change of parameters still verifies older hashes.

use std::sync::OnceLock;

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

fn argon2() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("valid Argon2 parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// A new PHC string for `password`, with a random salt. CPU-heavy: call it
/// from `spawn_blocking` in async code.
///
/// # Panics
///
/// Never in practice: hashing fails only for invalid parameters or salts, and
/// both are fixed here.
pub fn hash(password: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    argon2().hash_password(password.as_bytes(), &salt).expect("Argon2 hashing with fixed parameters").to_string()
}

/// Whether `password` matches the PHC string `phc`. A malformed `phc` never matches.
pub fn verify(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc).is_ok_and(|parsed| argon2().verify_password(password.as_bytes(), &parsed).is_ok())
}

/// Checks a sign-in attempt without blocking the runtime.
///
/// With no stored hash (no such account, or no password set yet) it still
/// verifies against a throwaway hash, so the answer takes as long as for a
/// real account and its timing does not tell which emails exist.
pub async fn check(password: String, stored: Option<String>) -> bool {
    tokio::task::spawn_blocking(move || match stored {
        Some(phc) => verify(&password, &phc),
        None => {
            let _ = verify(&password, decoy());
            false
        }
    })
    .await
    .unwrap_or(false)
}

fn decoy() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| hash("decoy password that no account has"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let phc = hash("correct horse battery staple");
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        assert!(verify("correct horse battery staple", &phc));
        assert!(!verify("correct horse battery stapl", &phc));
        assert_ne!(hash("correct horse battery staple"), phc, "every hash gets its own salt");
    }

    #[test]
    fn malformed_hashes_never_match() {
        assert!(!verify("anything", ""));
        assert!(!verify("anything", "plain text"));
    }

    #[tokio::test]
    async fn no_stored_hash_never_matches() {
        assert!(!check("decoy password that no account has".to_owned(), None).await);
    }
}
