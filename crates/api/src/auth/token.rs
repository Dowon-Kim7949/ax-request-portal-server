//! Bearer tokens: opaque random strings. The client never reads them; the
//! server keeps only their SHA-256.

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};

/// Marks a session token as this server's, which also lets secret scanners spot a leaked one.
pub const PREFIX: &str = "axs_";

/// Marks the token of a one-time setup link.
pub const LINK_PREFIX: &str = "axl_";

/// A new session token: the prefix and 256 random bits in unpadded base64url.
pub fn generate() -> String {
    generate_with(PREFIX)
}

/// A new token with `prefix` and 256 random bits in unpadded base64url.
pub fn generate_with(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// What the database stores for a token (`sessions`, `setup_links`).
pub fn digest(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_prefixed_random_and_url_safe() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert!(a.starts_with(PREFIX));
        assert_eq!(a.len(), PREFIX.len() + 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'), "{a}");
    }

    #[test]
    fn the_digest_is_sha256() {
        // sha256("abc")
        assert_eq!(digest("abc")[..4], [0xba, 0x78, 0x16, 0xbf]);
    }
}
