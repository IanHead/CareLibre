# Architecture Decision Records

Short, dated records of decisions that are expensive to reverse.

| # | Title | Status |
|---|---|---|
| [0001](0001-rust-axum-sqlx.md) | Rust, axum and sqlx for the server | Accepted |
| [0002](0002-postgresql-only.md) | PostgreSQL only | Accepted |
| [0003](0003-rest-openapi.md) | REST + OpenAPI, not GraphQL | Accepted |
| [0004](0004-organisation-multi-tenancy.md) | Organisation-scoped multi-tenancy | Accepted |
| [0005](0005-authentication.md) | Authentication: Argon2id + JWT + rotating refresh tokens | Accepted |
| [0006](0006-unified-organisations.md) | Unified organisations and user access | Accepted |
| [0007](0007-custom-fields.md) | Custom fields are metadata, never schema | Accepted |
| [0008](0008-permission-sets.md) | Configurable permission sets with capability checks | Accepted |
| [0009](0009-invitation-links.md) | One-time invitation links | Accepted |
| [0010](0010-csv-import.md) | CSV import with published templates and specs | Accepted |
| [0011](0011-rostering-suitability-graph.md) | Rostering suitability as declared constraints plus derived history | Accepted |
| [0012](0012-organisation-branding.md) | Organisation branding stored in PostgreSQL | Accepted |

## Adding an ADR

Copy the shape of an existing record: Context → Decision → Consequences →
Alternatives. Number sequentially. ADRs are immutable once accepted; supersede
rather than edit.
