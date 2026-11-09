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
- `relationships` is **two-sided**: each row stores `from_role` and `to_role`,
  so both endpoints carry their own label. A doctor/patient link is one row with
  `from_role = 'general_practitioner'` and `to_role = 'patient'`; reading it the
  other way gives patient/doctor without a second row. Each side is exactly one
  `contact` or one `organisation`, so contact↔contact, contact↔organisation and
  organisation↔organisation links all use the same table. Dates are optional.
  The `relationship_directions` view flattens each row into both directions,
  which is the shape graph clients consume.

Examples: a contact *is employee of* an organisation (`employee`/`employer`), a
contact *is guardian of* another contact (`guardian`/`person supported`), and a
health provider *is general practitioner of* a person
(`general_practitioner`/`patient`).

### Clients and workers

Client and worker are not contact types. They are relationships with the
organisation (the tenant root), read through the `contact_care_roles` view:

- **Client:** the contact is `care_recipient` and the organisation is
  `provider`.
- **Worker:** the contact is `employee` and the organisation is `employer`.

Only current relationships count (no `end_date`, or one today or later), in
either direction. A contact with both is dual role; one with neither is
"other". The contacts API exposes this as a read-only `role` on each contact
and as the `role` list filter (`client`, `worker`, `dual`, `other`, `all`).
Make someone a client or worker by adding the relationship; end it to stop.

### Relationship types

Roles are never free text. Each organisation keeps a catalogue of allowed role
pairs in `relationship_types` (for example `patient`/`general_practitioner`,
`mother`/`son`, `participant`/`support_coordinator`), grouped as care, family,
health, services, work or other. A relationship may use a pair in either
direction. Creating a relationship, changing its roles, correcting its roles
and importing relationships all require an active pair; otherwise the request
fails with a validation error.

New organisations start with a standard catalogue (including every gendered
family combination), and migration 0021 added every pair already in use.
Holders of `relationship_types.manage` (administrators by default) add pairs
and retire ones no longer wanted, through `/admin/relationship-types` or
`GET`/`POST /api/v1/relationship-types` and `PATCH /api/v1/relationship-types/{id}`.
Retired pairs stay on existing relationships and can still be corrected, but
cannot be chosen again. Typed roles are normalised, so "Support Coordinator"
becomes `support_coordinator`.

Contact search also matches the roles a contact currently holds toward
others, so searching "Support Coordinator" lists everyone acting as one.

### Relationships over time

Relationships are never edited silently or deleted. Each change is one of:

| Action | Use it when | Effect |
|---|---|---|
| Add | Something starts | New row; history starts with `added` |
| End | Offboarded, moved away, plan ended | Sets `end_date` (may be future) and a required `end_reason` |
| Transfer | Another party takes over | Ends the row the day before and starts a copy with the new party, linked by `previous_id` |
| Change role | Family member becomes guardian | Same, with new roles |
| Correct | It was recorded wrongly | Updates in place; recorded as `corrected` with before/after |
| Remove | It was never true | Sets `removed_at` and hides it everywhere; needs `relationships.remove` (administrators by default) |

End reasons: `offboarded`, `moved_away`, `plan_ended`, `deceased`,
`changed_provider`, `circumstances_changed`, `incident`, `other`.

`relationship_events` is the append-only history (who, when, why, before and
after); every action also writes a summary to `audit_log`. Notes on `incident`
events are returned only to holders of `relationships.incident_notes` (senior
management and house managers by default); others see that a note exists.
Ended relationships stay in `GET /relationships` with `status = "ended"` but
are left out of the network graph and of `contact_care_roles`.

## Organisations and branding

- `organisations` holds every organisation. The tenant root is `tenant_id = id`
  and carries the display name and slug.
- `organisation_branding` holds one optional logo per tenant root: bytes,
  content type, filename, SHA-256 ETag and the user who last changed it.
  Counterparties cannot carry branding; a trigger enforces tenant-root scope.
- The logo is capped at 2 MB and restricted to PNG, JPEG, WebP or GIF, detected
  from magic bytes. SVG is rejected. See ADR 0012.

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

- `sites` are physical places where care happens, optionally inside a division.
- `shifts` have a kind (`service_delivery`, `opening_hours`, `respite_program`),
  a time range, optional division, service and site, plus status.
- `shift_sessions` are sub-blocks of a shift and can hold their own assignments.
- `client_assignments` and `worker_assignments` attach a contact to a shift
  **or** to a session within it. A composite foreign key proves the session
  belongs to the same shift. Partial unique indexes prevent the same contact
  being assigned twice in the same scope.
- `worker_assignments` can carry an `outcome` (`positive`/`neutral`/`negative`)
  and a 1–5 `outcome_rating`.

## Rostering suitability

Suitability is a separate layer from care relationships. See ADR 0011.

- `roster_constraints` holds explicit `preferred`, `avoid` and `blocked`
  constraints for a worker against a contact, site, service or division.
  `blocked` is a hard exclusion; `avoid` is a penalty; `preferred` is a bonus.
  Contact constraints are symmetric, so a worker-worker conflict applies in both
  directions.
- `worker_client_history`, `worker_worker_history` and
  `worker_context_history` aggregate attended shifts, no-shows, cancellations,
  ratings, outcomes and recency from assignments.
- `GET /api/v1/shifts/{id}/suitability` scores candidates from those views and
  the active constraints, returning reasons, warnings and a blocked list. Scores
  are advisory; the endpoint never auto-assigns a worker.
- No score is stored on `relationships`. The evidence is append-only history, so
  recommendations can be recomputed and explained.

## Custom fields

User-defined metadata for any object, never schema changes:

- `custom_field_entities` lists the objects that may carry fields. Adding an
  object type is an `INSERT`, not a migration.
- `custom_field_definitions` holds the key, label, data type, required flag and
  validation rules (length, numeric range, regex, choice options).
- `custom_field_values` stores values as JSONB, unique per
  `(organisation_id, definition_id, entity_id)`.

A trigger rejects values whose JSON shape contradicts the definition, and checks
choice values against the configured options. Object type and data type are
immutable once values exist. Deleting a definition cascades its values; the UI
requires explicit confirmation. See [`custom-fields.md`](custom-fields.md) and
ADR 0007.

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
| Roster constraint has exactly one target | `CHECK (num_nonnulls(...) = 1)` |
| A worker cannot constrain themselves | `CHECK (target_contact_id <> worker_contact_id)` |
| Roster constraints are tenant-scoped | composite FKs to contacts, sites, services and divisions |
| Custom values match their declared type and options | trigger `validate_custom_field_value` |
| Custom field shape cannot change once values exist | trigger `guard_custom_field_definition_update` |

## Not yet modelled

These are intentionally deferred, and should arrive with the API that needs
them: rosters (as a named collection of shifts), recurring shift patterns,
availability, travel time, funding plans/budgets, invoices/claims, documents and
custom fields.
