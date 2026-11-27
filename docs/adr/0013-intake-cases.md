# ADR 0013 — Intake as a configurable case pipeline with independent contact lifecycle

- **Status:** Proposed
- **Date:** 2026-11-27
- **Supersedes:** none
- **Related:** [Intake API and behaviour contract](../intake.md), [Permissions](../permissions.md), [desktop implementation plan](https://github.com/IanHead/CareLibre-Desktop/blob/main/docs/intake-implementation-plan.md)

## Context

Intake is currently a desktop-only guided wizard. Drafts live in local SQLite and only
become server records when the wizard completes and `POST /api/v1/contacts` runs. There is
no shared list of enquiries, no referral or waitlist model, and no way for a coordinator to
see that a colleague is already handling a person.

The product also needs:

- a kanban board whose columns are configurable intake stages;
- different pipelines for clients (aged care, disability standard/emergency, fee for
  service), workers, other contacts and organisations, including sub-flows;
- a contact lifecycle status (Enquiry, Onboarding, Active, Suspended, Offboarding, Intake
  Cancelled, Offboarded) that is independent of the intake stage and may evolve later;
- per-stage ownership, alerts, field visibility, checklists and action restrictions;
- division/subdivision filtering;
- referrals and waitlists;
- server-side enforcement, because client-side hiding is not a control.

Contacts are deliberately untyped: client and worker identity comes from the
`contact_care_roles` view over two-sided relationships. The existing
`contacts.details.contact_status` value is free text and cannot carry a safe state machine.
The generic `audit_log` exists, but there is no notification facility.

## Decision

### 1. An intake case is the unit of pipeline work

`intake_cases` is a first-class org-scoped entity. A case holds the current stage, source,
division, assignee, stage data, referrals, waitlist state and history. The board renders
cases, not contacts or contacts' `details`.

The contact is created **at enquiry time**, in the same transaction as the case, so repeated
attempts reuse the same person. Duplicate matching uses normalised email, phone and
name + date of birth; the API returns candidates and the client can link one instead of
creating a duplicate. A contact may have several cases over time. At most one case may be
open per contact per flow, where "open" excludes both cancelled/archived cases and
successfully onboarded cases. This lets a person start a new intake after completing one
without creating a second person record.

Organisation intake links an organisation record instead of a contact. Client, worker and
other-contact flows require a contact.

### 2. Flows and stages are configuration; stage kinds are code

`intake_flows` groups a subject kind and variant (for example
`client / disability / emergency`) and may point at a parent flow for grouping. Stages are
ordered rows in `intake_stages`; administrators can rename, recolour, reorder, add and remove
them.

To keep server behaviour stable, every stage carries a fixed `stage_kind`:

`enquiry`, `eligibility`, `risk_assessment`, `service_planning`, `custom`, `onboarded`,
`cancelled`.

`custom` supports organisation-specific columns without a server release. `onboarded` and
`cancelled` are terminal. Stage labels and colours are configuration; the kinds are not.

Sub-flows are independent pipelines cloned from a parent or template. Changes to a parent do
not silently mutate children. The admin UI may offer an explicit "apply to sub-flows"
action. This avoids hidden coupling; a live shared stage library can be considered later if
duplication becomes painful.

### 3. The contact lifecycle is a separate axis

Intake stages are the kanban columns. `contacts.lifecycle_status` is a separate state
machine: `enquiry`, `onboarding`, `suspended`, `active`, `offboarding`, `intake_cancelled`,
`offboarded`. Statuses are shown as card pills and filters; they are not board columns and
may be extended later without changing intake.

Stages may carry an optional `on_enter_contact_status` nudge, but the lifecycle never
belongs to the case. A stage nudge is applied only when the transition is legal and must not
downgrade an active client starting a second intake. `suspended`, `offboarding` and
`offboarded` are changed only by an explicit lifecycle action with a reason.

All lifecycle changes write `contact_status_history` and `audit_log`.

### 4. Onboarded is complete, not closed

Reaching the `onboarded` stage sets `onboarded_at` and makes the case terminal for
transitions, but it does **not** set `closed_at`. The card remains visible in the Onboarded
column until archived. `closed_at` is reserved for cancellation/archival. The unique open
case constraint excludes both `onboarded_at` and `closed_at`, so a new intake can start once
the previous one has completed.

The onboarding transition is transactional and idempotent. It links the care/employment
relationship for the flow's subject kind and applies the lifecycle nudge. The role
relationship is not created at enquiry, so a cancelled enquiry does not classify someone as
a client or employee.

### 5. Policy is configuration, enforcement is server-side

Each stage can define:

- `visible_fields` and `required_fields`, referencing a server-owned field catalogue of
  built-in intake keys and `custom:<key>` values;
- allowed actions from a closed vocabulary: `roster.add`, `roster.remove`, `funding.add`,
  `funding.edit`, `funding.remove`, `profile.edit`, `relationships.update`;
- checklist template items, optionally gating transition;
- assignee rules: a named user, a permission-set queue/pool, or both, with
  `fixed`/`first_available`/`round_robin` strategies and notification flags.

Every mutating contact, relationship, funding and rostering endpoint that targets a contact
must call one shared intake policy helper. A missing guard is a security hole, so endpoint
tests enumerate the guarded actions. The desktop may hide or disable controls, but the
server is authoritative and returns a structured refusal.

### 6. Assignment and alerts are server-side

When a case enters a stage, the first resolvable assignee rule sets
`assignee_user_id`; additional rules with `notify` are watchers/backup recipients. Pools
resolve a member using the configured strategy and persist a round-robin cursor. Manual
reassignment requires `intake.assign` and is recorded as a case event.

Assignment creates an in-app notification when the recipient is not the actor. Deduplication
uses a case + stage + transition key. Email/SMS is deferred until the Communications
workspace exists; the notification row is the durable record an outbox can later deliver.

### 7. Configuration lives in the server admin UI

Flows, stages, fields, actions, checklists and assignees are managed at `/admin/intake`.
Clients consume the configuration through the API and do not ship a second configuration
surface. All configuration changes are audited.

### 8. Waitlists and referrals are part of the case model

Referrals are a case `source` plus referrer fields (contact or organisation, reference and
received time). A case can have waitlist entries while continuing through eligibility,
risk and planning. Waitlist is not a board column, so cases do not disappear from the
pipeline; it is a pill, a filter and eventually a dedicated view.

### 9. New capabilities

Add `intake.read`, `intake.write`, `intake.assign` and `intake.configure` to the capability
catalogue. The Intake persona receives read/write/assign; Senior Management and House
Manager receive read; Administrator receives all. Existing contact, relationship, funding
and rostering capabilities still apply in addition to the stage action policy.

## Consequences

**Positive**

- One shared, auditable pipeline for enquiries, referrals and waitlists.
- A person exists once, while each intake attempt is a separate case.
- Administrators can shape flows, stages, fields, checklists and actions without a release.
- Contact lifecycle and intake stages can evolve independently.
- Policy enforcement is centralised and testable; hiding controls is not the control.
- Onboarded cases remain visible without blocking a later intake.

**Negative**

- More tables and a new cross-cutting policy helper than a simple wizard.
- Stage configuration is powerful and can be misconfigured; defaults, validation and a
  preview are required.
- The field catalogue must be kept consistent between server and desktop during migration.
- Division filtering is not row-level authorisation; the existing permission model has no
  object-level scope.
- In-app alerts only in the first release; email depends on unbuilt communications
  infrastructure.

## Alternatives rejected

- **Board columns are the seven contact statuses.** Couples the pipeline to a small fixed
  lifecycle and prevents per-flow stages, sub-flows and stage-specific checklists.
- **Keep intake in `contacts.details`.** No queryable pipeline, no referential integrity, no
  shared board, and status becomes another free-text field.
- **Create the contact only at completion.** Duplicate people across abandoned attempts, no
  shared identity, and no way to search for an enquiry.
- **Separate case tables per subject kind.** Duplicates the state machine and makes the board,
  notifications and policy harder to share.
- **A single global stage list for every flow.** Cannot express aged care vs disability vs
  emergency sub-flows without overrides that are harder to reason about than cloned flows.
- **Client-side-only action restrictions.** Not a control; a modified client or direct API
  call bypasses it.
- **Email notifications in the first release.** The Communications workspace does not exist;
  in-app notifications deliver the immediate value and leave a clean outbox seam.
- **Make the case own the contact status.** Intake completion is not the only reason a
  contact becomes active, suspended or offboarded.

## Open items

- Whether enquiry-stage contacts should appear in Clients/Workers lists through a dedicated
  `prospect` relationship (preferred) or remain under All/lifecycle filtering until
  onboarding.
- Whether a live shared stage library should replace clone-on-create once several sub-flows
  share the same stages.
- Whether an `intake.override` capability is needed separately from `intake.configure` for
  transition overrides.
- Object-level division scoping for permission sets remains a separate phase.
