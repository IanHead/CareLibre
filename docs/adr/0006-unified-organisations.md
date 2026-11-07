# ADR 0006 — Unified organisations and user access

- **Status:** Accepted
- **Date:** 2026-11-07
- **Supersedes:** the separate `external_organisations` table and the name
  `organisation_members` introduced in ADRs 0001–0005.

## Context

The first domain migration split organisations into two tables:

- `organisations` — the tenant itself
- `external_organisations` — providers, brokers, agencies and other counterparties

That split implied two different things called "organisation" and forced
relationships to special-case each one. It also created a subtle modelling
question: is an internal branch an organisation, a division, or both?

Separately, `organisation_members` was ambiguous: it holds login accounts and
their roles, not care-model membership. Contacts belong to organisations through
roles, not through a membership table.

## Decision

1. **One `organisations` table for every organisation.** The tenant root is
   identified by `tenant_id = id`; internal branches and external counterparties
   are rows with `tenant_id` set to the tenant root. `slug` is required only for
   tenant roots.
2. **`external_organisations` is removed.** Existing rows are migrated into
   `organisations` with their identifiers preserved.
3. **`relationships` is the only connection between parties.** Endpoints are
   contacts or organisations; all four combinations are valid, and composite
   foreign keys keep both endpoints inside the relationship's tenant.
4. **`organisation_members` is renamed `organisation_users`.** It remains the
   authentication and authorisation table (which user may sign in to a tenant and
   with which role). It is explicitly not part of the care data model.

## Consequences

**Positive**

- One entity to reference from relationships, cost centres, reporting and the
  API.
- No special cases for "internal vs external" in queries or UI.
- Contact↔organisation roles are modelled once, in `relationships`, matching the
  untyped-contacts principle.
- The name `organisation_users` removes the ambiguity between access control and
  care-model membership.

**Negative**

- `organisations` now mixes tenancy metadata (`slug`, `edition`) with
  counterparty data (`kind`, `abn`). Tenant-only columns are simply null or
  unused for counterparties.
- Every query that lists counterparties must filter or join on `tenant_id`.
- Existing databases need a data migration; identifiers are preserved, so
  relationships remain valid.

## Alternatives rejected

- **Keep two tables behind a view:** hides the split but keeps duplicate IDs,
  duplicate constraints and special-case code.
- **Make users into contacts and model access as relationships:** elegant on
  paper, but couples authentication to mutable care data and complicates
  account recovery and auditing.
