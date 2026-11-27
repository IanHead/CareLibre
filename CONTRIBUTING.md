# Contributing

## Prerequisites

- Rust stable (see `rust-toolchain.toml`)
- PostgreSQL 16+
- Docker (optional, for the full stack)

## Workflow

```sh
make setup     # .env from .env.example
make test
make lint
```

CI runs only for releases. Run these checks locally before opening a pull request:

1. `cargo fmt --all`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. If you added a migration: run it against a real PostgreSQL and confirm it is
   forward-only and idempotent-safe (sqlx records checksums; never edit an
   applied migration).

## Conventions

- **Errors** go through `AppError`; never return a bare `StatusCode` with a
  message. Every non-2xx response uses the `{"error": {...}}` envelope.
- **Tenant scope** comes from `AuthContext`, never from a request body or query
  string.
- **Migrations** are numbered and forward-only (`NNNN_description.sql`). New
  tables that hold care data must include `organisation_id` and an index that
  starts with it.
- **OpenAPI** must stay accurate: annotate new handlers with `#[utoipa::path]`
  and register them in `api/openapi.rs`.
- **Secrets** never enter the repository. `.env` is ignored; CI uses throwaway
  values.

## Releases

Before tagging a release, add `docs/release-notes/vX.Y.Z.md` using
`docs/release-notes/TEMPLATE.md`. The release workflow refuses to publish without
the `New`, `Changed` and `Fixed` sections.

Write each bullet for a care provider, not a developer: describe what they can
now do, what works differently, or what no longer goes wrong. Avoid commit
hashes, file names, function names, API details and implementation jargon.

## Commit messages

Imperative mood, one logical change per commit:

```
add rate version effective-date constraint
```

## Decision records

Architectural changes need an ADR in `docs/adr/`. Supersede, don't rewrite.
