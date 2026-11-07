# Architecture

## Scope

This repository is the CareLibre **server**: a REST API over PostgreSQL. It is
one of four deliverables:

| Component | Repository | Status |
|---|---|---|
| Server | this repo | foundation |
| Desktop (Rust + egui/wgpu) | planned | — |
| Android | planned | — |
| iOS | planned | — |

The server must expose *all* functionality over the API; native clients are
never allowed to talk to PostgreSQL directly. The API is therefore the contract,
and OpenAPI is generated from the code that implements it.

## Runtime shape

```
client ──HTTPS──▶ axum router
                    │
                    ├── /healthz, /readyz          (infrastructure)
                    ├── /docs, /api-docs/*         (OpenAPI + Swagger UI)
                    └── /api/v1/*                  (versioned API)
                              │
                              ├── extractors: AuthContext (JWT)
                              ├── handlers: validation, DTO mapping
                              └── sqlx ──▶ PostgreSQL
```

- **One binary.** Configuration is environment-only; no config files to mount.
- **Migrations embedded** in the binary with `sqlx::migrate!`, applied at startup.
- **Stateless HTTP layer.** All session state is in PostgreSQL, so the server
  scales horizontally behind a load balancer.

## Multi-tenancy

Every domain row will belong to exactly one `organisation`. The current
foundation enforces this in application code:

- `organisation_members` maps users to organisations with a role.
- An access token is scoped to exactly one organisation (`org` claim).
- A user with memberships in several organisations must choose one at login.

**Next step:** add PostgreSQL row-level security keyed on
`current_setting('carelibre.organisation_id')`, so a handler bug cannot leak
across tenants. The application-level checks stay as defence in depth.

## Authentication

| Token | Format | Lifetime | Storage |
|---|---|---|---|
| Access | HS256 JWT | 15 min | client only |
| Refresh | 32 random bytes, base64url | 30 days | SHA-256 hash in `auth_sessions` |

- Passwords: Argon2id (default parameters), hashed on the blocking pool.
- Refresh tokens rotate on every use; reuse of a rotated token fails because the
  old row is revoked. (Reuse *detection* — revoking the whole family — is a
  planned hardening step.)
- Logout revokes the refresh token; access tokens expire naturally.
- Login failures return the same message for unknown users and bad passwords.

## Data model

The domain schema — divisions, cost centres and their configurable inheritance,
contacts, external organisations, relationships, services, rates and rate
versions, shifts, sessions and assignments — is described in
[`docs/data-model.md`](docs/data-model.md).

Foundation tables:

| Table | Purpose |
|---|---|
| `organisations` | Tenant. `slug`, display name, edition (`community`/`business`/`enterprise`) |
| `users` | Global accounts, unique case-insensitive email |
| `organisation_members` | User ↔ organisation with role |
| `auth_sessions` | Refresh-token hashes, expiry, revocation |
| `audit_log` | Append-only record of security-relevant actions |

Domain tables land in later migrations. Nothing is added speculatively: each
migration ships with the feature that needs it.

## Error contract

Every non-2xx response is:

```json
{ "error": { "code": "conflict", "message": "...", "details": { } } }
```

`code` is stable and machine-readable; `message` is safe to show to users.
Internal/database errors are logged server-side and returned as a generic
`internal_error` so nothing leaks.

## Observability

- `tracing` structured logs; JSON in production.
- `tower_http::trace` records method, path, status and latency per request.
- `/healthz` is process-only; `/readyz` verifies the database.
- No metrics endpoint yet — see roadmap.

## Deliberate non-goals (for now)

- Microservices. A modular monolith is the right shape until a component proves
  it needs independent scaling.
- Multi-database support. PostgreSQL-only is a documented decision (ADR 0002).
- GraphQL. REST + OpenAPI covers the clients and keeps caching/authz simple.
