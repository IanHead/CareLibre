# ADR 0005 — Authentication: Argon2id + JWT + rotating refresh tokens

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The API is consumed by native apps and machine integrations. It must work
without cookies, survive horizontal scaling, and allow revocation.

## Decision

- **Passwords:** Argon2id with default parameters, computed on the blocking pool.
- **Access tokens:** HS256 JWT, 15 minutes, carrying `sub`, `org`, `role`, `exp`,
  `jti`. Stateless verification; no database round-trip per request.
- **Refresh tokens:** 32 random bytes (base64url), stored only as SHA-256 hashes
  in `auth_sessions`. 30-day lifetime, **rotated on every refresh**; the old row
  is revoked.
- **Logout:** revokes the refresh token immediately. Access tokens expire.

## Consequences

**Positive**

- No server-side session lookup on the hot path.
- Refresh tokens are revocable, unlike bare JWTs.
- Stolen refresh tokens are single-use; reuse fails after rotation.
- Works identically for desktop, mobile and integrations.

**Negative**

- Role changes are visible only after the access token expires (≤ 15 min).
- HS256 means every server instance shares the signing secret; compromise of one
  instance compromises all.
- Revoked access tokens remain valid until expiry.

## Mitigations / next steps

- Keep access-token TTL short (already 15 min).
- Rotate `JWT_SECRET` with a key-id scheme if multi-instance key rotation is
  needed; consider EdDSA asymmeteric signing later.
- Add refresh-token **reuse detection** that revokes the whole session family.
- Consider a denylist keyed on `jti` only for high-risk operations.
