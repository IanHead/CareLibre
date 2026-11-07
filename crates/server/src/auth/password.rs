//! Argon2id password hashing.
//!
//! Hashing is CPU-bound, so the async wrappers move work to the blocking pool
//! to avoid stalling the Tokio runtime.

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::Argon2;

use crate::error::AppError;

/// Hash a password on the blocking pool.
pub async fn hash_password(password: String) -> Result<String, AppError> {
    tokio::task::spawn_blocking(move || hash_password_sync(&password))
        .await
        .map_err(|err| AppError::Internal(anyhow::Error::new(err)))?
}

/// Verify a password on the blocking pool. Returns `false` on any error,
/// including a malformed stored hash.
pub async fn verify_password(password: String, stored_hash: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password_sync(&password, &stored_hash))
        .await
        .unwrap_or(false)
}

pub fn hash_password_sync(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|err| AppError::Internal(anyhow::anyhow!("password hashing failed: {err}")))
}

#[must_use]
pub fn verify_password_sync(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(err) => {
            tracing::error!(error = %err, "stored password hash is malformed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_round_trip() {
        let hash = hash_password_sync("correct horse battery staple").unwrap();
        assert!(verify_password_sync("correct horse battery staple", &hash));
        assert!(!verify_password_sync("wrong password", &hash));
    }

    #[test]
    fn rejects_malformed_hashes() {
        assert!(!verify_password_sync("anything", "not-a-phc-string"));
    }
}
