# ADR 0002 — PostgreSQL only

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The brief lists "PostgreSQL, MariaDB or MySQL", noting Azure runs PostgreSQL.
Supporting multiple engines from day one doubles migration work, forces the
lowest-common-denominator schema, and makes advanced features unusable.

## Decision

**Target PostgreSQL 16+ only.** Do not write a compatibility layer.

Features we intend to rely on:

- `jsonb` for custom field definitions and audit metadata
- Range types (`tstzrange`) for shift/session overlap detection
- Row-level security for tenant isolation
- Partial and expression indexes
- `GENERATED ALWAYS AS IDENTITY`, `uuid` handling, `timestamptz` semantics

## Consequences

**Positive**

- One dialect, one migration set, one CI database.
- Roster overlap and effective-dated rates can be enforced by the database.
- Azure Database for PostgreSQL works without changes.

**Negative**

- Providers who only run MySQL/MariaDB cannot self-host without changing engines.
- A future MariaDB port would be a project, not a flag.

## Mitigations

- Deployment via `docker compose` ships PostgreSQL, so the supported path is
  always available.
- If demand appears, a MySQL port can be evaluated as a separate ADR with a
  measured cost. Nothing in the HTTP layer depends on the engine.
