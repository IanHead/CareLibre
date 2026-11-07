# ADR 0004 — Organisation-scoped multi-tenancy

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

One deployment serves many care providers (tenants). The Community Edition is
self-hosted per provider; the Business/Enterprise editions may host several.
The security requirement is absolute: no request may read or write another
organisation's data.

## Decision

1. Every domain row carries `organisation_id`.
2. A user is linked to organisations through `organisation_members`, with a role.
3. An **access token is scoped to exactly one organisation**. Users who belong to
   several must choose one at login (`organisation_slug`).
4. Handlers derive `organisation_id` from the token, never from the request body.
5. Application-level checks are the first line; **PostgreSQL row-level security**
   on `organisation_id` is the second and will be enabled as domain tables land.

## Consequences

**Positive**

- Tenant isolation is explicit and testable at the token boundary.
- A single connection pool serves all tenants; no per-tenant databases.
- Switching organisations in the UI is a token exchange, not a data merge.

**Negative**

- Cross-organisation reporting (e.g. a bookkeeper serving several providers)
  requires multiple sessions or a future aggregated view.
- Every new table must remember `organisation_id` and an RLS policy; enforced by
  a migration checklist and, later, a CI check.

## Alternatives rejected

- **Database per tenant:** strong isolation, but upgrades and connection
  management become operationally expensive for the free tier.
- **Global user with implicit "current org" header:** invites confused-deputy
  bugs; the token scope is harder to forge than a header.
