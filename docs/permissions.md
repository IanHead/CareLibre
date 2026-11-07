# Permissions

Configurable permission sets with out-of-the-box security personas.

Built-in role names are not the authorisation model. Screens, API endpoints and
database operations check **capabilities**; personas are presets that group
capabilities for common care-provider jobs. A provider can edit the presets or
create their own sets, with a small number of non-negotiable guardrails.

## Implementation status

Phase 1 is shipped:

- capability catalogue, permission sets, assignments and `is_owner` in migration
  0008;
- OOTB personas seeded per organisation (including existing organisations);
- Administrator is immutable and the last owner cannot be demoted (DB triggers);
- capabilities resolved per request for the admin UI;
- `/admin/permission-sets` and per-user access assignment;
- navigation and pages gated by capability, with `403` on direct access.

Still outstanding:

- contacts API endpoints check `contacts.read`/`contacts.write`; new endpoints
  must follow the same pattern;
- House Manager is read-only **org-wide** until scoped assignments land;
- invitations forward the chosen sets (invitation design is agreed, not built);
- the legacy `role` column remains for JWT/display compatibility and must not be
  used for new checks.

## Principles

1. **Deny by default.** A user with no permission set can sign in but do nothing.
2. **Capabilities, not role names.** `can("rosters.write")`, never
   `role == "manager"`.
3. **Additive.** A user may hold several sets; effective capabilities are the
   union.
4. **Server-enforced.** Hiding a button is convenience; every endpoint checks the
   capability again.
5. **Owner bypass is explicit and protected.** Exactly one or more `owner`
   accounts hold every capability; the last owner cannot be removed, disabled or
   demoted.
6. **Administrator is immutable.** The OOTB Administrator set always contains
   every capability and cannot be edited or deleted.
7. **Every change is audited.** Who changed which set, and what was added or
   removed.

## Capability catalogue

`domain.action` naming. Keep this list authoritative; new features add
capabilities here.

### People and parties

| Capability | Meaning |
|---|---|
| `contacts.read` | View people records |
| `contacts.write` | Create/edit people records |
| `relationships.read` | View relationships between contacts/organisations |
| `relationships.write` | Create/edit relationships |
| `organisations.read` | View organisations in the tenant |
| `organisations.write` | Create/edit organisations |

### Care catalogue

| Capability | Meaning |
|---|---|
| `divisions.read` / `divisions.write` | Organisational units |
| `cost_centres.read` / `cost_centres.write` | Accounting dimensions |
| `services.read` / `services.write` | Funded services |
| `rates.read` / `rates.write` | Rates and effective-dated versions |
| `custom_fields.manage` | Define and delete custom fields |

### Rostering

| Capability | Meaning |
|---|---|
| `rosters.read` | View shifts, sessions and assignments |
| `rosters.write` | Create/edit shifts, sessions and assignments |
| `rosters.approve` | Approve/publish rosters |

### Finance

| Capability | Meaning |
|---|---|
| `finance.read` | View invoices, claims and financial reports |
| `finance.write` | Create/edit invoices and claims |

### Human resources

| Capability | Meaning |
|---|---|
| `hr.read` | View worker compliance, availability and HR records |
| `hr.write` | Maintain worker compliance and HR records |

### Administration and reporting

| Capability | Meaning |
|---|---|
| `users.manage` | Invite users, assign permission sets, deactivate accounts |
| `permission_sets.manage` | Create/edit permission sets |
| `settings.manage` | Organisation settings, branding, editions |
| `audit.read` | Audit log and session list |
| `reports.read` | Operational and financial reports |
| `data.import` | Run CSV imports |
| `integrations.manage` | Accounting/comms integrations |
| `comms.manage` | SMS/email templates and providers |
| `schema.view` | Database explorer |

## Out-of-the-box personas

| Capability | Rostering | Intake | HR | Senior Mgmt | House Manager | Finance | Administrator |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `contacts.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `contacts.write` | — | ✓ | ✓ | — | — | — | ✓ |
| `relationships.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `relationships.write` | — | ✓ | — | — | — | — | ✓ |
| `organisations.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `organisations.write` | — | ✓ | — | — | — | — | ✓ |
| `divisions.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `divisions.write` | — | — | — | — | — | — | ✓ |
| `cost_centres.read` | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ |
| `cost_centres.write` | — | — | — | — | — | — | ✓ |
| `services.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `services.write` | — | — | — | — | — | — | ✓ |
| `rates.read` | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ |
| `rates.write` | — | — | — | — | — | ✓ | ✓ |
| `custom_fields.manage` | — | — | — | — | — | — | ✓ |
| `rosters.read` | ✓ | — | ✓ | ✓ | ✓ | ✓ | ✓ |
| `rosters.write` | ✓ | — | — | — | — | — | ✓ |
| `rosters.approve` | — | — | — | ✓ | — | — | ✓ |
| `finance.read` | — | — | — | ✓ | — | ✓ | ✓ |
| `finance.write` | — | — | — | — | — | ✓ | ✓ |
| `hr.read` | — | — | ✓ | ✓ | — | — | ✓ |
| `hr.write` | — | — | ✓ | — | — | — | ✓ |
| `users.manage` | — | — | — | — | — | — | ✓ |
| `permission_sets.manage` | — | — | — | — | — | — | ✓ |
| `settings.manage` | — | — | — | — | — | — | ✓ |
| `audit.read` | — | — | — | ✓ | — | — | ✓ |
| `reports.read` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `data.import` | — | ✓ | — | — | — | — | ✓ |
| `integrations.manage` | — | — | — | — | — | ✓ | ✓ |
| `comms.manage` | — | — | — | — | — | — | ✓ |
| `schema.view` | — | — | — | — | — | — | ✓ |

### Persona intent

| Persona | In one line |
|---|---|
| **Rostering** | Builds and publishes rosters; reads the people and catalogue it needs; no finance or administration |
| **Intake** | Creates and links people and organisations; imports referral data; no rostering |
| **Human Resources** | Maintains worker records and compliance; reads rosters; no finance |
| **Senior Management** | Read access across the organisation, approves rosters, sees audit and financial summaries |
| **House Manager** | A worker who runs a house: read-only access to rosters and records, nothing destructive |
| **Finance** | Owns rates, invoices, claims and accounting integrations; reads rosters for billing |
| **Administrator** | Full access to everything, immutable preset |

**Owner** (the account created at registration) is not a permission set. It
bypasses capability checks entirely and is protected by the last-owner rule.

### Known limitation: House Manager scope

House Manager is read-only but currently **organisation-wide**, not limited to
one house/division. Object-level scoping needs a scope dimension on assignments
(e.g. `scope_division_id`) and capability checks that receive the target record.
That is planned for Phase 2; until then, do not present House Manager as
house-scoped in customer-facing material.

## Data model

```sql
-- Global catalogue, seeded by migration.
CREATE TABLE capabilities (
    key          text PRIMARY KEY,
    category     text NOT NULL,
    label        text NOT NULL,
    description  text NOT NULL,
    sort_order   integer NOT NULL DEFAULT 0
);

-- Per-tenant sets. OOTB personas are seeded for every organisation.
CREATE TABLE permission_sets (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    description      text,
    persona_key      text,          -- rostering, intake, hr, senior_management,
                                    -- house_manager, finance, administrator
    is_system        boolean NOT NULL DEFAULT false,
    is_editable      boolean NOT NULL DEFAULT true,
    sort_order       integer NOT NULL DEFAULT 100,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    UNIQUE (organisation_id, id),
    UNIQUE (organisation_id, name)
);

CREATE TABLE permission_set_capabilities (
    organisation_id   uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    capability        text NOT NULL REFERENCES capabilities (key),
    PRIMARY KEY (permission_set_id, capability),
    FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE CASCADE
);

-- Additive: a user may hold several sets.
CREATE TABLE organisation_user_permission_sets (
    organisation_id   uuid NOT NULL,
    user_id           uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    PRIMARY KEY (organisation_id, user_id, permission_set_id),
    FOREIGN KEY (organisation_id, user_id)
        REFERENCES organisation_users (organisation_id, user_id) ON DELETE CASCADE,
    FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE RESTRICT
);
```

`organisation_users` gains `is_owner boolean NOT NULL DEFAULT false`. The legacy
`role` column stays during the transition (JWT/display), but **must not** be used
for new authorisation checks.

### Seeding

- A function `carelibre_seed_permission_sets(organisation_id)` inserts the seven
  personas and their capabilities.
- Called by a trigger on `organisations` insert, and backfilled for existing
  organisations in the migration.
- Administrator is inserted with `is_system = true, is_editable = false` and
  every capability.
- Other personas are `is_system = true, is_editable = true` so providers can
  adjust them, while `persona_key` keeps their identity for the UI.

### Guardrails (database-enforced where possible)

| Risk | Guard |
|---|---|
| Locking out the last owner | Trigger on `organisation_users`: cannot clear `is_owner` or deactivate the final owner |
| Editing Administrator into something useless | `is_editable = false`; trigger rejects capability changes for system/immutable sets |
| Deleting a set in use | FK `ON DELETE RESTRICT`; UI requires reassignment first |
| Self-elevation | Only `permission_sets.manage` and `users.manage` can change assignments; owner bypass applies |
| Silent privilege changes | Audit entry per set/assignment change with before/after capability diff |
| Orphaned capabilities | FK to `capabilities`; deleting a capability from the catalogue requires a migration and review |

## Effective permissions

```
if user.is_owner: every capability
else: union(capabilities of every assigned permission_set)
```

Resolved server-side on each request (or cached with a version counter). Do not
put the full capability list in the JWT: permission changes should take effect
immediately, and tokens are long-lived enough to matter.

`AuthContext` gains:

```rust
pub struct AuthContext {
    pub user_id: Uuid,
    pub organisation_id: Uuid,
    pub is_owner: bool,
    pub capabilities: CapabilitySet,
}

impl AuthContext {
    pub fn can(&self, capability: Capability) -> bool { ... }
}
```

Every handler that mutates or reads restricted data calls `auth.require(...)`,
which returns `403` with `error.code = "forbidden"`.

## Admin UI

- **`/admin/permission-sets`** — list personas and custom sets, capabilities
  grouped by category, create/clone/edit/delete.
- **`/admin/users`** — assign one or more sets per user, show effective
  capability count, warn when a user has no sets.
- **Immutable set** — Administrator shows a lock and cannot be edited.
- **Diff before save** — show added/removed capabilities; require confirmation
  for removals that affect active users.
- **Empty set warning** — a user with no set can sign in but sees nothing.
- **House Manager note** — display the org-wide read-only caveat until scoping
  ships.

## Integration with onboarding

The onboarding "Review roles and permission sets" task becomes concrete:

1. Show the seven personas with one-line descriptions and the number of assigned
   users.
2. Offer "Assign a persona" for each user missing one.
3. Confirm the Administrator set still has a second member (feeds the
   `access.second_admin` detection).

Detection for `access.second_admin` becomes: at least two active users who are
owners or hold the Administrator set.

## Implementation plan

**Phase 1 — model and enforcement**

1. Migration: capability catalogue, permission sets, assignments, `is_owner`,
   seeding function and triggers.
2. Backfill: owner → `is_owner`; `admin` → Administrator; `manager` → Rostering +
   Intake; `staff` → House Manager; `viewer` → House Manager. (No role column
   reads for authorisation afterwards.)
3. Rust `Capability` enum and `CapabilitySet`, resolved with the admin session.
4. Replace `can_manage_organisation()` checks with capability checks page by
   page: users, sessions, schema, custom fields, settings, onboarding.
5. Admin UI for sets and user assignment, with audit entries.
6. Tests: deny-by-default, additive sets, owner bypass, immutable Administrator,
   last-owner protection, tenant isolation, page access.

**Phase 2 — scope and invitations**

7. Optional scope on assignments (`scope_division_id`) and capability checks that
   receive the target record; House Manager becomes house-scoped.
8. Invitations carry the intended permission sets.
9. API endpoints enforce the same capabilities.

**Phase 3 — polish**

10. Clone/edit flows with diff preview, usage counts, "users affected" warnings.
11. Persona recommendations based on actual usage; export of the permission
    matrix for auditors.

## Acceptance criteria

- A user with no sets can authenticate but every restricted page returns 403.
- House Manager can read rosters and records and receives 403 on every write.
- Finance can change rates and run finance reports but cannot edit contacts.
- Rostering can edit rosters but cannot see finance.
- Administrator can do everything; the set cannot be edited or deleted.
- Removing the last owner is impossible, including by direct SQL.
- Changing a set takes effect on the next request, without re-login.
- Every set/assignment change appears in `audit_log`.
- No tenant can reference another tenant's set (composite FKs).
