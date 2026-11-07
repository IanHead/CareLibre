# Data model

Structural overview of the CareLibre schema. The admin UI renders the same
information live at **`/admin/schema`**.

## Principles

1. **Every domain row is tenant-scoped.** `organisation_id` is on every table
   and composite foreign keys (`organisation_id, id`) make cross-tenant
   references impossible at the database level.
2. **Contacts are untyped.** There is no `client`/`worker` column. Roles come
   from relationships and from assignment rows.
3. **Effective dates are half-open.** Rate versions and relationship periods use
   `[from, to)` semantics.
4. **Derived data is computed, not duplicated.** A record's cost centre is
   resolved on demand from its direct assignment or its configured parents.

## Object map

```
organisations (tenant)
 ├── organisation_members ── users
 ├── divisions ──▶ divisions (parent)
 ├── cost_centres ──▶ cost_centres (parent)
 │    └── cost_centre_assignments  (any entity_type + entity_id)
 ├── contacts
 ├── external_organisations
 ├── relationships  (contact ⇄ contact, contact ⇄ external org, org ⇄ org)
 ├── services ──▶ divisions
 │    └── rates
 │         └── rate_versions
 └── shifts ──▶ divisions
      ├── shift_sessions
      ├── client_assignments  ──▶ contacts
      └── worker_assignments  ──▶ contacts
```

## Cost centres

- `cost_centres` is a tree (`parent_id`) scoped to the organisation, with an
  optional mapping to an external accounting system (`external_system`,
  `external_code`).
- `cost_centre_assignments` holds the **direct** cost centre for any record:
  `(organisation_id, entity_type, entity_id) → cost_centre_id`. One per record.
- `record_parents` holds generic child → parent links. A child can have several
  parents of different types, but only one of each type.
- `cost_centre_inheritance_rules` defines the ordered lookup path per entity
  type: `(entity_type, inherits_from, priority)`.
- `carelibre_resolve_cost_centre(organisation_id, entity_type, entity_id)`
  resolves the effective cost centre: direct assignment first, then walk the
  configured parents, up to 16 levels.

Example configuration:

```sql
-- A shift looks for: its own assignment -> session -> roster -> division.
INSERT INTO cost_centre_inheritance_rules
    (organisation_id, entity_type, inherits_from, priority)
VALUES
    (:org, 'shift', 'session', 1),
    (:org, 'shift', 'roster',  2),
    (:org, 'shift', 'division', 3);
```

`record_parents` must mirror the real links when the application creates them
(e.g. `('shift', shift_id) → ('division', division_id)`). The resolver is
`STABLE`, so it can be used in reports and views.

### Known entity types

`division`, `contact`, `external_organisation`, `service`, `rate`, `shift`,
`shift_session`, `client_assignment`, `worker_assignment`. The column is free
text so new types need no migration; keep this list in sync when adding one.

## Contacts, organisations and relationships

- `contacts` holds people. Deliberately no `type` column: a participant can be a
  worker on another shift, and a family member can be a guardian, a nominee and
  a driver without duplicating the record.
- `external_organisations` holds counterparties (providers, brokers, health
  services, agencies). Not to be confused with `organisations` (tenants).
- `relationships` is directed and typed (`relationship_type`, free text) with an
  optional `inverse_name`. Exactly one entity per side, so all four
  combinations are valid. Dates are optional.

## Divisions, services, rates

- `divisions` are hierarchical organisational units.
- `services` belong to a division and map to a funding scheme:
  `ndis`, `aged_care` or `other`, with a funding code.
- `rates` belong to a service, carry a name and a unit of measure
  (`hour`, `day`, `each`, `kilometre`, …). Units are free text.
- `rate_versions` are effective-dated prices:
  `amount`, `currency`, `effective_from`, `effective_to` (exclusive).
  A trigger rejects overlapping ranges, and a partial unique index allows only
  one open-ended version per rate. This avoids requiring the `btree_gist`
  extension on managed PostgreSQL.

## Rostering

- `shifts` have a kind (`service_delivery`, `opening_hours`, `respite_program`),
  a time range, optional division and status.
- `shift_sessions` are sub-blocks of a shift and can hold their own assignments.
- `client_assignments` and `worker_assignments` attach a contact to a shift
  **or** to a session within it. A composite foreign key proves the session
  belongs to the same shift. Partial unique indexes prevent the same contact
  being assigned twice in the same scope.

## Rostering rules enforced by the database

| Rule | Mechanism |
|---|---|
| Shift ends after it starts | `CHECK (ends_at > starts_at)` |
| Session belongs to its shift | composite FK `(organisation_id, shift_id, session_id)` |
| No duplicate assignment per shift/session | partial unique indexes |
| No overlap between rate versions | trigger using `daterange &&` |
| Tenant separation | composite `(organisation_id, id)` foreign keys |
| One cost centre per record | unique `(organisation_id, entity_type, entity_id)` |

## Not yet modelled

These are intentionally deferred, and should arrive with the API that needs
them: rosters (as a named collection of shifts), recurring shift patterns,
availability, travel time, funding plans/budgets, invoices/claims, documents and
custom fields.
