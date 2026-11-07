# Onboarding

How a care provider goes from "server is running" to "first shift delivered",
and how the product guides them there.

The onboarding dashboard is the *surface*; this document also defines the
process, the tasks, how completion is detected, and what must exist before each
task can be honest.

## Goals

1. **Time to value.** A solo provider should reach a first completed shift in
   under an hour. A migrating provider should know exactly what to import and in
   what order.
2. **Safety first.** Backups, production settings and role design are tasks, not
   assumptions. Nothing in onboarding should encourage importing real client data
   before the operator has verified the basics.
3. **Resumable, non-blocking.** Setup is never a hard gate. The app remains fully
   usable; progress is remembered, and skipping is allowed with a reason.
4. **Honest tasks.** A task links only to a screen that exists. Features that are
   still being built are marked as such rather than offering dead links.
5. **No PII on the checklist.** Progress uses counts and existence checks, never
   record contents.

## Personas and scenarios

| Scenario | Who | Shape of the journey |
|---|---|---|
| **Solo self-serve** | Owner-operator, Community Edition | Fast path: profile → invite nobody → division → one service + rate → 5 contacts → first shift. 20–40 minutes |
| **Small team rollout** | Owner + admin + coordinator | Add admins, assign roles, review divisions, import or enter staff, pilot roster |
| **Migration** | Provider leaving another CRM | Import contacts/relationships/services/rates, reconcile counts, parallel run, cut over |
| **Multi-site / enterprise** | Operations manager, IT | Divisions and cost centres, per-division rates, permission sets, integrations, staged rollout |
| **Broker / support coordinator** | External organisation user (future) | Lighter setup focused on relationships and referrals, not rosters |

Mode matters: a fresh-start provider should not be pushed towards CSV import, and
a migrating provider should not be asked to hand-key 400 contacts first.

## The journey

```
0. First login            ─ already done (register created org + owner)
1. Organisation profile   ─ identity, ABN, contact details, locale/timezone
2. Security & access      ─ second admin, roles/permission sets, production checks, backups
3. Structure              ─ divisions, cost centres
4. Catalogue              ─ services, rates, rate versions, custom fields review
5. People                 ─ contacts, external organisations, relationships (manual or import)
6. Rostering pilot        ─ one division, one service, first shift, assignments, completed shift
7. Integrations & comms   ─ accounting, SMS/email (later)
8. Go live                ─ readiness review, backup verification, staff access, sign-off
```

Stages 1–6 are the minimum useful product. Stages 7–8 can be deferred but must
stay visible so nobody thinks setup is finished when it is not.

### Migration mode

When the operator chooses "I am migrating from another system", the People and
Catalogue stages expand:

1. Download the templates and specs (`/admin/import`).
2. Import divisions and cost centres.
3. Import organisations, contacts and relationships.
4. Import services, rates and rate versions.
5. Reconcile counts against the source system.
6. Run a pilot shift with imported data.
7. Keep the old system read-only for a parallel period, then cut over.

Fresh-start providers see the same stages but with manual entry as the primary
action and import marked optional.

## Task catalogue

`A` = detected automatically from data, `M` = operator marks done.
Required tasks count towards "ready to go live".

### 1. Organisation

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `org.profile` | Review organisation name, ABN and contact details | M | yes | owner | 2 minutes; feeds documents and invoices |
| `org.timezone` | Confirm timezone and locale | M | yes | owner | Rosters and effective dates depend on it |

### 2. Security and access

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `access.second_admin` | Add a second owner or administrator | A | yes | owner | Detected as two active owners or Administrator-set holders |
| `access.roles` | Review personas and assign permission sets | M | yes | owner | Seven OOTB personas; see [`permissions.md`](permissions.md) |
| `access.team` | Invite the rest of the team | A | no | owner/admin | Detected as more than the initial admin; one-time links |
| `security.production` | Confirm production settings | M | yes | owner | `APP_ENV=production`, unique JWT secret, TLS, no default secrets |
| `security.backups` | Verify database backups | M | yes | owner | Test a restore, not just a schedule |

### 3. Structure

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `structure.division` | Create at least one division | A | yes | owner/admin | Divisions drive service ownership and cost centres |
| `structure.cost_centres` | Add cost centres | A | no | owner/admin | Only needed for accounting mapping |

### 4. Catalogue

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `catalogue.service` | Add a service mapped to a funding code | A | yes | owner/admin | NDIS or aged care |
| `catalogue.rate` | Add a rate with an effective version | A | yes | owner/admin | Effective-dated; overlap is rejected |
| `catalogue.custom_fields` | Review custom fields | M | no | owner/admin | Resist adding speculative fields |

### 5. People

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `people.contacts` | Add or import clients and workers | A | yes | owner/admin/manager | Contacts are untyped; roles come from relationships |
| `people.relationships` | Link contacts and organisations | A | yes | owner/admin/manager | Guardians, brokers, employers, doctors |
| `people.import` | Import from a CSV export | A | no | owner/admin | Only offered in migration mode / once import exists |

### 6. Rostering pilot

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `roster.shift` | Create the first shift | A | yes | manager | Any kind: service delivery, opening hours, respite |
| `roster.assignments` | Assign a client and a worker | A | yes | manager | Duplicate assignment is blocked |
| `roster.completed` | Run and complete one pilot shift | M | yes | manager | The real proof the configuration works |

### 7. Integrations and communications (later)

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `integrations.accounting` | Connect Xero or QuickBooks | M | no | owner | Not built |
| `comms.sms` | Configure SMS/email provider | M | no | owner | Not built |

### 8. Go live

| Key | Task | C | Required | Roles | Notes |
|---|---|---|---|---|---|
| `readiness.audit` | Review users, sessions and audit log | M | yes | owner | Confirm only intended accounts have access |
| `readiness.signoff` | Mark the organisation live | M | yes | owner | Requires all required tasks resolved |

## Dashboard

Route: `/admin/onboarding`, linked from the sidebar as **Setup** with a progress
badge, and as a card on `/admin`.

```
┌ Setup ─────────────────────────────────────────────── 6 of 13 required ┐
│ ███████████░░░░░░░░░░░░  46%           Next: Add a service              │
│                                                                         │
│ 1. Organisation                                                         │
│   ✓ Review organisation details            verified  2 min              │
│   ○ Confirm timezone and locale            Mark done 1 min              │
│                                                                         │
│ 2. Security and access                                                  │
│   ✓ Add a second administrator             detected                     │
│   ○ Review roles and permission sets       Mark done 5 min              │
│   ○ Confirm production settings            Open guide 10 min            │
│   ○ Verify database backups                Mark done 30 min             │
│ ...                                                                     │
│                                                         [Skip] [Help]    │
└─────────────────────────────────────────────────────────────────────────┘
```

Behaviour:

- **Sections** mirror the stages above; completed sections collapse.
- **Statuses:** `todo`, `done`, `skipped`. Required tasks and optional tasks are
  counted separately. A skipped required task is shown in amber, not green.
- **Automatic tasks** show "Detected automatically" when the query returns true;
  the operator cannot hack the tick, but can see *what* was detected.
- **Manual tasks** have `Mark done` and `Skip`; skipping asks for a one-line
  reason (stored for the audit trail, not shown to other users).
- **Next up** is the first incomplete required task in stage order, skipping tasks
  whose prerequisites are incomplete.
- **Deep links** go to the screen that resolves the task. If the screen is not
  built yet, the task is labelled `Coming soon` and the dashboard section is
  collapsed by default.
- **Completion:** when every required task is resolved, show a summary and a
  single "Go live" confirmation. The card can then be dismissed from the
  dashboard; `/admin/onboarding` remains available.
- **First login:** show a banner on the dashboard and a nav badge, but do not
  force a redirect. Forced tours are the fastest way to make operators click
  through without reading.
- **Role aware:** only `owner` and `admin` see the checklist. `manager` sees the
  roster tasks assigned to them (future).

## State model (proposed)

Task definitions live in code (a versioned catalogue) so wording and detection can
improve without migrations. Only state is persisted:

```sql
CREATE TABLE organisation_onboarding (
    organisation_id  uuid PRIMARY KEY REFERENCES organisations (id) ON DELETE CASCADE,
    mode             text NOT NULL DEFAULT 'guided'
                     CHECK (mode IN ('guided', 'migration')),
    catalogue_version integer NOT NULL DEFAULT 1,
    started_at       timestamptz NOT NULL DEFAULT now(),
    completed_at     timestamptz,
    dismissed_at     timestamptz,
    updated_at       timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE onboarding_task_states (
    organisation_id  uuid NOT NULL REFERENCES organisation_onboarding (organisation_id) ON DELETE CASCADE,
    task_key         text NOT NULL CHECK (char_length(task_key) BETWEEN 1 AND 64),
    status           text NOT NULL CHECK (status IN ('done', 'skipped')),
    reason           text,
    marked_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    marked_at        timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, task_key)
);
```

- No row means `todo`.
- Automatic tasks are computed on read; they are not written to state (except
  optionally as an audit event).
- `catalogue_version` lets a release add tasks: new tasks appear as `todo`, and
  the UI can announce "2 new setup steps".
- `dismissed_at` hides the dashboard card but keeps the nav badge until required
  tasks are done.

## Automatic detection

One `EXISTS`/`count` per auto task, all tenant-scoped and index-friendly:

| Task | Check |
|---|---|
| `access.second_admin` | `count(organisation_users where role in ('owner','admin')) >= 2` |
| `structure.division` | `EXISTS divisions` |
| `structure.cost_centres` | `EXISTS cost_centres` |
| `catalogue.service` | `EXISTS services` |
| `catalogue.rate` | `EXISTS rate_versions` |
| `people.contacts` | `count(contacts) >= 1` (migration mode: >= 10, or import batch succeeded) |
| `people.relationships` | `EXISTS relationships` |
| `roster.shift` | `EXISTS shifts` |
| `roster.assignments` | `EXISTS client_assignments OR worker_assignments` |
| `people.import` | `EXISTS import_batches WHERE status = 'committed'` (after import ships) |

Manual tasks cannot be detected honestly (backups, pilot shift, role review) and
must stay manual. Do not infer "done" from related activity — a false tick is
worse than an empty checkbox.

## Permission sets

See [`permissions.md`](permissions.md) for the capability catalogue and the persona matrix. Summary:

1. **Owner** — the registration account, bypasses checks, protected by the
   last-owner rule.
2. **Capability-based permission sets** — additive sets assigned to users. Seven
   OOTB personas: Rostering, Intake, Human Resources, Senior Management,
   House Manager, Finance, Administrator. Administrator is immutable; the others
   can be edited.
3. **Scopes (later)** — org-wide today; per-division/house scoping is required
   before House Manager is marketed as "their house" only.

## Data import

Import is its own feature; onboarding links to it and waits for a committed batch.

Wizard:

1. **Choose object** — contacts, external organisations, relationships,
   divisions, cost centres, services, rates/versions.
2. **Upload CSV** — size limit, UTF-8, header row.
3. **Map columns** — saved mappings per organisation; custom fields included by
   key.
4. **Dry run** — show would-create / would-update / rejected, with per-row errors
   downloadable as CSV.
5. **Choose conflict policy** — create-only, update-by-external-id, or fail on
   duplicate. Never silently overwrite.
6. **Commit** — staged rows applied in a transaction per chunk; audit entry with
   counts and file hash.
7. **Summary** — counts, skipped rows, link to the next import.

Safety requirements: PII stays server-side, no sample rows in logs, imports are
idempotent by `external_id`, and a failed chunk rolls back without leaving
half-linked relationships.

## Dependencies and honest sequencing

| Capability | Needed for | Status |
|---|---|---|
| Contact/division/service/rate CRUD screens or API | most auto tasks | **not built** |
| Invitation links | `access.second_admin`, `access.team` | **designed, not built** ([invitations.md](invitations.md)) |
| CSV import + templates | migration mode, `people.import` | **designed, not built** ([import.md](import.md)) |
| Configurable permission sets | `access.roles`, scoped personas | **designed, not built** ([permissions.md](permissions.md)) |
| Restore verification guide | `security.backups` | docs only |
| Accounting/SMS integrations | stage 7 | **not built** |
| Production checks (env, secret length, TLS) | `security.production` | partially detectable |

Because the CRUD screens do not exist yet, the dashboard should ship in two steps:

- **Phase 1 — framework.** State tables, catalogue, progress, manual ticks,
  automatic checks for data that exists today, honest "coming soon" labels.
- **Phase 2 — useful dashboard.** As each CRUD/importer screen lands, wire the
  task to it and move it out of "coming soon".

Building the full dashboard before the CRUD screens would produce a checklist of
dead links, which is worse than no checklist.

## Implementation plan

**Phase 1 (can start now)**

- Migration: `organisation_onboarding`, `onboarding_task_states`.
- Rust task catalogue with stable keys, sections, required flags, roles, estimates,
  detection queries and destination links.
- `/admin/onboarding` with progress, sections, manual done/skip, reason capture.
- Dashboard card + sidebar badge; owner/admin only.
- Automatic checks for users, divisions, cost centres, custom fields and any data
  that exists today.
- Audit events for manual completion and skip.
- Tests: progress calculation, required-vs-optional, skip reason, tenant scoping,
  auto-detection with seeded rows.

**Phase 2 (with core CRUD)**

- Wire each task to its screen; remove "coming soon".
- Add "create the thing" inline actions where reasonable.
- First-login banner; completion summary.

**Phase 3 (with import)**

- Migration mode, import task, reconciliation checklist, parallel-run reminder.

**Phase 4 (with permission sets and integrations)**

- Role review against permission sets, integration tasks, go-live gating.

## Acceptance criteria (Phase 1)

- A new organisation sees 0% and a next task, without any console errors.
- Completing a manual task updates progress for all admins immediately.
- Skipping a required task keeps the organisation below 100% and records the reason.
- Automatic tasks flip to done when the corresponding rows appear, with no manual
  write.
- No task leaks data from another organisation.
- The dashboard is reachable from the sidebar and dashboard card, and never blocks
  the rest of the admin area.

## Success measures

- Median time from first login to first shift.
- Percentage of organisations reaching "ready to go live" within 7 days.
- Drop-off by stage (where people stop).
- Import error rate and time to reconcile.
- Support tickets tagged "setup" per 100 new organisations.

## Decisions needed

1. ~~**Permission sets:** fixed roles or configurable?~~ **Resolved:** configurable
   capability sets with seven OOTB personas; see [`permissions.md`](permissions.md).
2. ~~**Invitations:** email delivery or shareable one-time link?~~ **Resolved:**
   one-time shareable link, shown once and hashed at rest; see
   [`invitations.md`](invitations.md) and ADR 0009.
3. ~~**Import scope:** CSV first or connectors?~~ **Resolved:** CSV first with
   downloadable templates and column specs; connectors later reuse the pipeline.
   See [`import.md`](import.md) and ADR 0010.
4. ~~**Forced vs optional:**~~ **Resolved:** non-blocking banner and sidebar
   badge, no forced first-run wizard.
