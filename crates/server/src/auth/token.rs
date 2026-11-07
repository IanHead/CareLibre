//! JWT access tokens (HS256).
//!
//! Tokens are intentionally short-lived. They carry the user's organisation and
//! role so that most requests need no database round-trip; the trade-off is that
//! role changes take effect within one access-token lifetime.

use std::time::Duration;

use chrono::Utc;
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    /// Subject: user id.
    pub sub: Uuid,
    /// Active organisation id.
    pub org: Uuid,
    /// Role within that organisation.
    pub role: String,
    /// Token use: always `access` (refresh tokens are opaque, not JWTs).
    pub typ: String,
    pub iat: i64,
    pub exp: i64,
    /// Unique token id, useful for tracing/revocation lists.
    pub jti: Uuid,
}

pub fn issue_access_token(
    secret: &str,
    user_id: Uuid,
    organisation_id: Uuid,
    role: &str,
    ttl: Duration,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = Utc::now();
    let claims = Claims {
        sub: user_id,
        org: organisation_id,
        role: role.to_owned(),
        typ: "access".to_owned(),
        iat: now.timestamp(),
        exp: now.timestamp() + i64::try_from(ttl.as_secs()).unwrap_or(900),
        jti: Uuid::now_v7(),
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub fn verify_access_token(
    secret: &str,
    token: &str,
) -> Result<Claims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_required_spec_claims(&["exp", "sub", "iat"]);
    validation.validate_aud = false;
    // Tolerate modest clock skew between instances, nothing more.
    validation.leeway = 30;

    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;

    if data.claims.typ != "access" {
        return Err(jsonwebtoken::errors::Error::from(
            jsonwebtoken::errors::ErrorKind::InvalidToken,
        ));
    }

    Ok(data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-secret-that-is-at-least-32-characters-long";

    #[test]
    fn round_trips_a_token() {
        let user = Uuid::now_v7();
        let org = Uuid::now_v7();

        let token =
            issue_access_token(SECRET, user, org, "admin", Duration::from_secs(60)).unwrap();
        let claims = verify_access_token(SECRET, &token).unwrap();

        assert_eq!(claims.sub, user);
        assert_eq!(claims.org, org);
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.typ, "access");
    }

    #[test]
    fn rejects_a_wrong_secret() {
        let token = issue_access_token(
            SECRET,
            Uuid::now_v7(),
            Uuid::now_v7(),
            "staff",
            Duration::from_secs(60),
        )
        .unwrap();

        assert!(verify_access_token("another-secret-at-least-32-characters", &token).is_err());
    }

    #[test]
    fn rejects_expired_tokens() {
        let now = Utc::now();
        let claims = Claims {
            sub: Uuid::now_v7(),
            org: Uuid::now_v7(),
            role: "staff".to_owned(),
            typ: "access".to_owned(),
            iat: (now - chrono::Duration::hours(2)).timestamp(),
            exp: (now - chrono::Duration::hours(1)).timestamp(),
            jti: Uuid::now_v7(),
        };

        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .unwrap();

        assert!(verify_access_token(SECRET, &token).is_err());
    }
}
