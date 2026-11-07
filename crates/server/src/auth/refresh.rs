//! Opaque refresh/session token generation and hashing.
//!
//! Refresh tokens are random, never stored in plaintext, and are used by both
//! the JSON API and the browser admin session.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Return `(token_for_client, sha256_hex_for_storage)`.
pub fn generate() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let hash = hash(&token);
    (token, hash)
}

pub fn hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_unique_and_hashable() {
        let (a, hash_a) = generate();
        let (b, hash_b) = generate();

        assert_ne!(a, b);
        assert_ne!(hash_a, hash_b);
        assert_eq!(hash(&a), hash_a);
        assert_eq!(hash_a.len(), 64);
    }
}
