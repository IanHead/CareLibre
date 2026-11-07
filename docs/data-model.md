# Data model

Structural overview of the CareLibre schema. The admin UI renders the same
information live at **`/admin/schema`**.

## Principles

1. **Every domain row is tenant-scoped.** `organisation_id` is on every table
   and composite foreign keys (`organisation_id, id`) make cross-tenant
   references impossible at the database level.
2. **There is one organisation table.** The tenant itself, internal branches and
   external counterparties (providers, brokers, agencies) are all rows in
   `organisations`, scoped by `tenant_id`. The tenant root points at itself.
3. **Relationships are the only glue between parties.** Contacts and
   organisations link to each other exclusively through `relationships`.
4. **Contacts are untyped, organisations are not classified by table.** There is
   no `client`/`worker` column and no separate external-organisation table.
   `organisations.kind` is a free-text descriptor, not an enum.
5. **Effective dates are half-open.** Rate versions and relationship periods use
   `[from, to)` semantics.
6. **Derived data is computed, not duplicated.** A record's cost centre is
   resolved on demand from its direct assignment or its configured parents.

## Object map

```
organisations (tenant root, tenant_id = id)
 └── organisations (internal branches, external counterparties; tenant_id = root)
      ├── relationships ──▶ contacts | organisations
      ├── divisions ──▶ divisions (parent)
      ├── cost_centres ──▶ cost_centres (parent)
      │    └── cost_centre_assignments  (any entity_type + entity_id)
      ├── contacts
      ├── services ──▶ divisions
      │    └── rates
      │         └── rate_versions
      └── shifts ──▶ divisions
           ├── shift_sessions
           ├── client_assignments  ──▶ contacts
           └── worker_assignments  ──▶ contacts

organisation_users ── users      (authentication only, not the care model)
```

## Organisations

`organisations` holds every organisation:

| Column | Meaning |
|---|---|
| `tenant_id` | Tenant root that owns this row. The root has `tenant_id = id` |
| `slug` | URL slug, required for tenant roots, optional for counterparties |
| `kind` | Free text: `provider`, `broker`, `health`, `agency`, … |
| `abn` | Australian Business Number, 11 digits, optional |
| `email`, `phone`, `website`, `notes` | Contact details |
| `is_active` | Soft-disable without deleting history |

A new row with no `tenant_id` is treated as a tenant root by
`set_organisation_tenant()`; anything created inside a tenant sets `tenant_id`
explicitly.

`organisation_users` is **not** part of this model. It records which login
accounts may access a tenant and with which role (`owner`, `admin`, `manager`,
`staff`, `viewer`). Contact-to-organisation membership is expressed with
`relationships`.

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

`organisation`, `division`, `contact`, `service`, `rate`, `shift`,
`shift_session`, `client_assignment`, `worker_assignment`. The column is free
text so new types need no migration; keep this list in sync when adding one.

## Contacts and relationships

- `contacts` holds people. Deliberately no `type` column: a participant can be a
  worker on another shift, and a family member can be a guardian, a nominee and
  a driver without duplicating the record.
- `relationships` is directed and typed (`relationship_type`, free text) with an
  optional `inverse_name`. Each side is exactly one `contact` or one
  `organisation`, so contact↔contact, contact↔organisation and
  organisation↔organisation links all use the same table. Dates are optional.

Examples: a contact *works for* an organisation, an organisation *refers to*
another organisation, a contact is *guardian of* another contact, a person is a
*doctor at* a health provider.

## Divisions, services, rates

- `divisions` are hierarchical organisational units inside a tenant.
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

## Rules enforced by the database

| Rule | Mechanism |
|---|---|
| Shift ends after it starts | `CHECK (ends_at > starts_at)` |
| Session belongs to its shift | composite FK `(organisation_id, shift_id, session_id)` |
| No duplicate assignment per shift/session | partial unique indexes |
| No overlap between rate versions | trigger using `daterange &&` |
| Tenant separation | composite `(organisation_id, id)` foreign keys |
| One cost centre per record | unique `(organisation_id, entity_type, entity_id)` |
| Relationship endpoints are in the same tenant | composite FKs to `organisations (tenant_id, id)` |

## Not yet modelled

These are intentionally deferred, and should arrive with the API that needs
them: rosters (as a named collection of shifts), recurring shift patterns,
availability, travel time, funding plans/budgets, invoices/claims, documents and
custom fields.
