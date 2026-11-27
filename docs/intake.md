# Intake API and behaviour contract

- **Status:** draft for implementation
- **Version:** v1 (under `/api/v1`)
- **Related:** [ADR 0013](adr/0013-intake-cases.md), [Permissions](permissions.md), [desktop implementation plan](https://github.com/IanHead/CareLibre-Desktop/blob/main/docs/intake-implementation-plan.md)

This document is the wire contract for the intake board, cases, configuration, assignments,
checklists and lifecycle. It is written for both server and client implementers. The ADR
explains why; this document explains exactly what.

---

## 1. Scope

In scope:

- configurable intake flows and stages;
- enquiries, referrals and waitlists as intake cases;
- contact creation/matching at enquiry and role linking on onboarding;
- stage transitions, required fields, checklists and action policy;
- per-stage assignment and in-app notifications;
- contact lifecycle status and history;
- division/subdivision filtering;
- server admin configuration.

Out of scope for v1:

- email/SMS delivery (the Communications workspace does not exist yet);
- object-level division authorisation (divisions are a filter, not row-level security);
- a shared live stage library (sub-flows are cloned);
- organisation-specific lifecycle statuses beyond the fixed set (the set is extensible by
  migration, not by configuration);
- external referral intake APIs/webhooks;
- document attachments and consent management.

---

## 2. Terminology

| Term | Meaning |
|---|---|
| **Flow** | A complete intake pipeline for one subject kind and variant, e.g. `client / disability / emergency` |
| **Stage** | An ordered column in a flow, e.g. Risk assessment. Every stage has a fixed `kind` |
| **Case** | One attempt at intake for a contact or organisation. The kanban card |
| **Contact lifecycle** | The person's operational status (`enquiry`, `onboarding`, `active`, …). Independent of the stage |
| **Open case** | A case that is neither onboarded nor closed. Used for duplicate prevention |
| **Onboarded** | A terminal stage that sets `onboarded_at`. It remains visible in the Onboarded column until archived |
| **Closed** | A cancelled or archived case (`closed_at` set). Hidden unless `include_closed=true` |
| **Action policy** | The per-stage allow/deny set for profile, relationship, funding and rosters actions |
| **Field catalogue** | Built-in intake keys plus the organisation's custom fields, addressed as `custom:<key>` |

---

## 3. Object model

```
intake_flows 1 ─── * intake_stages 1 ─── * intake_stage_actions
     │                    │            ├── * intake_stage_assignees
     │                    │            └── * intake_stage_checklist_items
     │
     └── * intake_cases 1 ─── * intake_case_checklist_items
                  │      ├── * intake_case_events
                  │      └── * intake_waitlist_entries
                  │
                  ├── 1 contact (person flows) ─── * contact_status_history
                  └── 0..1 organisation (organisation flows)

intake_cases 1 ─── * notifications (via assignment/transition)
```

Contacts are untyped. The case's `subject_kind` identifies the pipeline. The client or
worker role relationship is created only when the case reaches its role-link stage
(default: `onboarded`), not at enquiry.

---

## 4. Authentication, tenancy and permissions

Every endpoint requires `Authorization: Bearer <access_token>` from
`POST /api/v1/auth/login`. All rows are scoped to the caller's organisation; an id from
another tenant is a `404`, not a `403`.

| Capability | Grants |
|---|---|
| `intake.read` | Flows, board, cases, timeline, waitlists, assignee picker |
| `intake.write` | Create/update/transition/archive cases, toggle checklist items, manage waitlists |
| `intake.assign` | Change the assignee of any case |
| `intake.configure` | Create/update/archive flows, stages, actions, checklists and assignees |

Existing capabilities still apply in addition to the stage action policy:

- profile edits require `contacts.write`;
- relationship edits require `relationships.write`;
- funding edits require `finance.write`;
- roster edits require `rosters.write`.

A stage policy can deny an action that the user's capability would otherwise allow. The
stage policy never grants an action on its own.

Creating a case with a new contact requires both `intake.write` and `contacts.write`.
Linking an existing contact requires only `intake.write`. The onboarding transition links
the care/employment relationship and therefore also requires `relationships.write`; if the
actor lacks it, the transition is refused rather than producing a half-onboarded contact.

Owner accounts bypass capability checks. Administrator permission sets are immutable and
contain every capability.

---

## 5. Errors

Every non-2xx response uses the existing envelope:

```json
{
  "error": {
    "code": "validation_failed",
    "message": "Complete the required checks before moving on.",
    "details": {
      "reason": "intake_transition_blocked",
      "missing_fields": ["date_of_birth"],
      "incomplete_checklist": ["…"]
    }
  }
}
```

- `code` is stable and generic (`validation_failed`, `conflict`, `forbidden`, …).
- `details.reason` is the intake-specific reason.
- `message` is safe to show to a user.
- Clients must not parse the message.

### Status codes

| Status | When |
|---|---|
| `400` | Malformed JSON, bad query parameter |
| `401` | Missing/expired token |
| `403` | Capability missing (`forbidden`) |
| `404` | Unknown or cross-tenant id |
| `409` | Version conflict, duplicate contact, existing open case, action blocked by stage policy, stage in use |
| `422` | Validation failure or blocked transition (missing fields/checklist) |
| `500` | Internal error; details are not leaked |

### Error reasons

| Reason | Status | Meaning |
|---|---|---|
| `contact_match` | `409` | Candidate contacts exist; link one or force duplicate |
| `open_case_exists` | `409` | The contact already has an open case for this flow |
| `version_conflict` | `409` | `version` is stale; refetch and retry |
| `intake_transition_blocked` | `422` | Required fields or checklist items are incomplete |
| `invalid_transition` | `422` | Target stage is not reachable from the current stage |
| `intake_action_blocked` | `409` | Stage policy denies a contact action |
| `stage_in_use` | `409` | Cannot delete a stage with cases |
| `flow_in_use` | `409` | Cannot hard-delete a flow with cases; archive instead |
| `invalid_field_key` | `422` | A configured field key is not in the catalogue |
| `invalid_action_key` | `422` | An unknown action key |
| `contact_required` | `422` | A person flow was created without a contact |
| `lifecycle_transition_invalid` | `422` | Illegal contact status transition |

---

## 6. Common JSON types

### Flow

```json
{
  "id": "0193f2d0-8a5e-7c3b-9f1a-2b6c7d8e9f01",
  "key": "client_disability_standard",
  "name": "Disability — standard",
  "subject_kind": "client",
  "variant": "disability",
  "parent_flow_id": "0193f2d0-8a5e-7c3b-9f1a-2b6c7d8e9f00",
  "description": "Standard disability intake",
  "division_id": null,
  "is_default": true,
  "is_active": true,
  "sort_order": 30,
  "stages": []
}
```

`subject_kind`: `client`, `worker`, `other_contact`, `organisation`.

`variant` is free text with suggested values `standard`, `aged_care`, `disability`,
`emergency`, `fee_for_service`.

### Stage

```json
{
  "id": "0193f2d0-8a5e-7c3b-9f1a-2b6c7d8e9f10",
  "key": "risk_assessment",
  "name": "Risk assessment",
  "kind": "risk_assessment",
  "color": "amber",
  "sort_order": 30,
  "is_terminal": false,
  "on_enter_contact_status": "onboarding",
  "sla_hours": 48,
  "checklist_gate": true,
  "visible_fields": [
    "first_name", "family_name", "date_of_birth", "risk_summary", "custom:ndis_number"
  ],
  "required_fields": ["date_of_birth", "risk_summary"],
  "actions": {
    "roster.add": false,
    "roster.remove": false,
    "funding.add": false,
    "funding.edit": false,
    "funding.remove": false,
    "profile.edit": true,
    "relationships.update": true
  },
  "assignees": [],
  "checklist": []
}
```

Rules:

- `kind` is immutable once a case has entered the stage.
- `is_terminal` is read-only and true only for `onboarded` and `cancelled`.
- `on_enter_contact_status` may be `enquiry`, `onboarding`, `active`, `intake_cancelled`
  or `null`. It is a nudge, not ownership.
- `visible_fields` and `required_fields` must reference catalogue keys. Required fields
  should be a subset of visible fields.
- `actions` always contains all seven keys.

### Stage assignee

```json
{
  "id": "0193f2d0-8a5e-7c3b-9f1a-2b6c7d8e9f20",
  "user": { "id": "…", "full_name": "Alex Nguyen", "email": "alex@example.com" },
  "permission_set": null,
  "strategy": "fixed",
  "notify": true,
  "sort_order": 10
}
```

A queue rule uses `user: null` and a permission set:

```json
{
  "user": null,
  "permission_set": { "id": "…", "name": "Intake" },
  "strategy": "round_robin",
  "notify": true,
  "sort_order": 20
}
```

Exactly one of `user` / `permission_set` is present. `strategy` is `fixed` for a named
user, and `first_available` or `round_robin` for a pool. Multiple rules are ordered; the
first resolvable rule is the primary assignee, and later `notify=true` rules are watchers.

### Checklist template item

```json
{
  "id": "…",
  "label": "Confirm identity documents",
  "description": "Photo ID or two secondary documents",
  "is_required": true,
  "sort_order": 10
}
```

### Case summary (board card)

For organisation flows, `contact_id` and `lifecycle_status` are `null` and the card shows
the organisation name.

```json
{
  "id": "…",
  "display_name": "Mia Chen",
  "contact_id": "…",
  "flow": {
    "id": "…",
    "key": "client_disability_standard",
    "name": "Disability — standard",
    "subject_kind": "client",
    "variant": "disability"
  },
  "stage_id": "…",
  "stage": {
    "id": "…",
    "key": "risk_assessment",
    "name": "Risk assessment",
    "kind": "risk_assessment"
  },
  "lifecycle_status": "onboarding",
  "division": { "id": "…", "name": "Northern", "parent_id": "…" },
  "assignee": { "id": "…", "full_name": "Alex Nguyen" },
  "priority": 1,
  "source": "referral",
  "waitlisted": false,
  "checklist": {
    "done": 2,
    "total": 5,
    "required_done": 2,
    "required_total": 3
  },
  "stage_entered_at": "2026-11-27T02:00:00Z",
  "stage_due_at": "2026-11-29T02:00:00Z",
  "updated_at": "2026-11-27T04:30:00Z",
  "version": 4
}
```

### Case detail

`ContactSummary` in the examples below is the existing contact summary subset (`id`,
`display_name`, `email`, `phone`, `lifecycle_status`, `role`). It is not a new DTO.

`CaseSummary.display_name` is denormalised for board performance and is refreshed when the
linked contact's display name changes.

```json
{
  "case": {
    "…": "all Case summary fields, plus:",
    "data": {
      "first_name": "Mia",
      "family_name": "Chen",
      "date_of_birth": "1990-04-12",
      "custom:ndis_number": "430000000"
    },
    "referral": {
      "source": "referral",
      "referrer_contact_id": null,
      "referrer_organisation_id": "…",
      "reference": "REF-1042",
      "received_at": "2026-11-26T23:10:00Z"
    },
    "created_by": { "id": "…", "full_name": "Sam Lee" },
    "created_at": "2026-11-27T02:00:00Z",
    "onboarded_at": null,
    "closed_at": null,
    "cancellation_reason": null
  },
  "contact": {
    "id": "…",
    "display_name": "Mia Chen",
    "email": "mia@example.com",
    "phone": "0400 000 000",
    "lifecycle_status": "onboarding",
    "role": "other"
  },
  "flow": { "…": "Flow with stages" },
  "stage": { "…": "Stage including actions and checklist" },
  "visible_fields": ["first_name", "family_name", "date_of_birth", "risk_summary"],
  "required_fields": ["date_of_birth", "risk_summary"],
  "checklist": [
    {
      "id": "…",
      "label": "Confirm identity documents",
      "is_required": true,
      "completed_at": null,
      "completed_by": null,
      "note": null,
      "sort_order": 10
    }
  ],
  "allowed_transitions": [
    { "id": "…", "key": "service_planning", "name": "Service planning", "kind": "service_planning" }
  ],
  "action_policy": {
    "profile.edit": { "allowed": true, "reason": null },
    "relationships.update": { "allowed": true, "reason": null },
    "funding.add": { "allowed": false, "reason": "Funding is locked during risk assessment." }
  },
  "waitlist": []
}
```

### Timeline event

```json
{
  "id": 1042,
  "event_type": "transitioned",
  "actor": { "id": "…", "full_name": "Sam Lee" },
  "from_stage": { "id": "…", "name": "Eligibility" },
  "to_stage": { "id": "…", "name": "Risk assessment" },
  "metadata": { "reason": null },
  "created_at": "2026-11-27T04:30:00Z"
}
```

`event_type`: `created`, `updated`, `transitioned`, `assigned`, `checklist_completed`,
`checklist_reopened`, `waitlisted`, `cancelled`, `onboarded`, `archived`, `reopened`,
`action_denied`.

### Notification

```json
{
  "id": 9001,
  "kind": "intake.assigned",
  "title": "Mia Chen moved to Risk assessment",
  "body": "You are the assignee for this stage.",
  "entity_type": "intake_case",
  "entity_id": "…",
  "read_at": null,
  "created_at": "2026-11-27T04:30:00Z"
}
```

### Field catalogue entry

```json
{
  "key": "date_of_birth",
  "label": "Date of birth",
  "data_type": "date",
  "section": "Personal details",
  "source": "builtin",
  "options": [],
  "required_by_default": false
}
```

Custom fields use `"source": "custom"` and a key of `custom:<key>`.

---

## 7. Configuration endpoints

Configuration endpoints require `intake.read` for reads and `intake.configure` for writes.
Every write is audited in `audit_log`.

### `GET /api/v1/intake/flows`

Query:

| Parameter | Type | Notes |
|---|---|---|
| `subject_kind` | repeatable/comma | Filter by subject kind |
| `include_inactive` | bool | Default `false` |
| `division_id` | uuid | Include flows scoped to this division |
| `include_children` | bool | Include child-division scoped flows. Default `true` |

Response `200`:

```json
{ "items": [Flow, …] }
```

### `POST /api/v1/intake/flows`

```json
{
  "key": "client_disability_standard",
  "name": "Disability — standard",
  "subject_kind": "client",
  "variant": "disability",
  "parent_flow_id": "…",
  "description": "Standard disability intake",
  "division_id": null,
  "is_default": false,
  "clone_from_flow_id": "…"
}
```

- `key` is stable, lower snake case, 2–64 characters, unique per organisation.
- If `clone_from_flow_id` is present, stages and their policy are copied.
- Otherwise the flow is created with the default six-stage pipeline:
  Enquiry, Eligibility, Risk assessment, Service planning, Onboarded, Cancelled.
- `parent_flow_id` must reference a flow in the same organisation.
- `is_default` is unique per `(subject_kind, variant)`; setting it clears the previous
  default.
- Response `201`: `Flow`.

### `PATCH /api/v1/intake/flows/{id}`

Partial update of `name`, `variant`, `parent_flow_id`, `description`, `division_id`,
`is_default`, `is_active`, `sort_order`. `key` and `subject_kind` are immutable.
Response `200`: `Flow`.

### `DELETE /api/v1/intake/flows/{id}`

Archives the flow (`is_active = false`). If no cases exist, the flow and its stages may be
deleted; the server may choose to archive instead. Archiving does not close existing cases:
the picker hides the flow, but its board remains readable by id until the cases are
archived. Returns `204`.

### `POST /api/v1/intake/flows/{id}/stages`

```json
{
  "key": "risk_assessment",
  "name": "Risk assessment",
  "kind": "risk_assessment",
  "color": "amber",
  "on_enter_contact_status": "onboarding",
  "sla_hours": 48,
  "checklist_gate": true,
  "visible_fields": ["first_name", "risk_summary"],
  "required_fields": ["risk_summary"]
}
```

- The stage is appended at the end of the flow.
- `visible_fields` and `required_fields` must be in the field catalogue.
- `required_fields` must be a subset of `visible_fields`.
- `kind` is one of `enquiry`, `eligibility`, `risk_assessment`, `service_planning`,
  `custom`, `onboarded`, `cancelled`.
- A flow must have exactly one first `enquiry` stage, exactly one `onboarded` stage and
  exactly one `cancelled` stage. `custom` stages are non-terminal. Terminal stages must be
  last.
- Response `201`: `Stage`.

### `PATCH /api/v1/intake/stages/{id}`

Partial update of `name`, `color`, `on_enter_contact_status`, `sla_hours`,
`checklist_gate`, `visible_fields`, `required_fields`. `kind` is immutable once a case has
entered the stage. `is_terminal` is read-only. Response `200`: `Stage`.

### `DELETE /api/v1/intake/stages/{id}`

Deletes an unused, non-first, non-terminal stage. If cases have entered it, returns `409`
with reason `stage_in_use`. Returns `204`.

### `POST /api/v1/intake/flows/{id}/stages/reorder`

```json
{ "stage_ids": ["…", "…", "…"] }
```

Must contain every stage in the flow exactly once. The server rejects a reorder that puts a
non-`enquiry` stage first or places a non-terminal stage after a terminal stage. Response
`200`: `Flow`.

### `PUT /api/v1/intake/stages/{id}/actions`

Replaces the full action set:

```json
{
  "allowed": ["profile.edit", "relationships.update"]
}
```

Valid keys: `roster.add`, `roster.remove`, `funding.add`, `funding.edit`, `funding.remove`,
`profile.edit`, `relationships.update`. Unknown keys return `422 invalid_action_key`.
Response `200`: `Stage`.

### `PUT /api/v1/intake/stages/{id}/checklist`

Replaces the stage template:

```json
{
  "items": [
    {
      "label": "Confirm identity documents",
      "description": "Photo ID or two secondary documents",
      "is_required": true
    }
  ]
}
```

Existing case checklist items keep their snapshot; only future entries are affected.
Response `200`: `Stage`.

### `PUT /api/v1/intake/stages/{id}/assignees`

Replaces the stage assignment rules:

```json
{
  "assignees": [
    {
      "user_id": "…",
      "permission_set_id": null,
      "strategy": "fixed",
      "notify": true,
      "sort_order": 10
    },
    {
      "user_id": null,
      "permission_set_id": "…",
      "strategy": "round_robin",
      "notify": true,
      "sort_order": 20
    }
  ]
}
```

Exactly one of `user_id` / `permission_set_id` is set per rule. `fixed` is only valid with
a user; `first_available` and `round_robin` are only valid with a pool. `first_available`
selects the active pool member with the fewest open cases (ties by user id);
`round_robin` rotates through the pool using a persisted cursor. Where several rules are
present, the first resolvable rule sets the primary assignee; later `notify=true` rules are
watchers. A watcher pool notifies every active member and does not consume the round-robin
cursor. Changing the rules does not reassign cases already in the stage; it applies when a
case next enters. Response `200`: `Stage`.

### `GET /api/v1/intake/field-catalog`

Response `200`:

```json
{
  "fields": [
    {
      "key": "first_name",
      "label": "First name",
      "data_type": "text",
      "section": "Personal details",
      "source": "builtin",
      "options": [],
      "required_by_default": false
    },
    {
      "key": "custom:ndis_number",
      "label": "NDIS number",
      "data_type": "text",
      "section": "Funding",
      "source": "custom",
      "options": [],
      "required_by_default": false
    }
  ]
}
```

The built-in catalogue is owned by the server. Custom entries are the organisation's active
`contact` custom fields. A desktop compatibility test asserts that every key in its legacy
wizard exists in this catalogue until the wizard is removed.

### `GET /api/v1/intake/assignees`

Response `200`:

```json
{
  "users": [
    { "id": "…", "full_name": "Alex Nguyen", "email": "alex@example.com" }
  ],
  "permission_sets": [
    { "id": "…", "name": "Intake", "persona_key": "intake" }
  ]
}
```

Returns active organisation users and non-archived permission sets usable as assignment
pools.

---

## 8. Board and case listing

### `GET /api/v1/intake/board`

The board is **single-flow** in v1. The client chooses a flow with a selector or tabs.
Showing several flows merged by stage kind is deliberately deferred, because stage ids,
fields, actions and checklists are flow-specific.

Query:

| Parameter | Type | Notes |
|---|---|---|
| `flow_id` | uuid | Required |
| `division_id` | uuid | Optional |
| `include_descendants` | bool | Default `true`; includes child divisions |
| `assignee_user_id` | uuid / `me` / `unassigned` | Optional |
| `lifecycle_status` | repeatable/comma | Optional |
| `search` | string | Matches display name and contact email/phone |
| `include_closed` | bool | Default `false` |
| `limit_per_stage` | int | Default `30`, max `100` |

Response `200`:

```json
{
  "flow": { "…": "Flow summary" },
  "columns": [
    {
      "stage": { "…": "Stage summary" },
      "count": 12,
      "cards": [CaseSummary, …],
      "next_cursor": "opaque-cursor-or-null"
    }
  ],
  "generated_at": "2026-11-27T05:00:00Z"
}
```

- `count` is the total in the column for the current filters, not the number of cards
  returned.
- Cards are ordered by `priority DESC`, `stage_entered_at ASC`, `id ASC`.
- `next_cursor` is opaque. Pass it to `GET /intake/cases` with the same filters to fetch
  more for that column.
- Onboarded cases are included while `closed_at IS NULL`. Cancelled/archived cases require
  `include_closed=true`.
- If `flow_id` is unknown or belongs to another organisation, `404`.

### `GET /api/v1/intake/cases`

Same filters as the board, plus:

| Parameter | Type | Notes |
|---|---|---|
| `stage_id` | uuid | Required for column paging |
| `cursor` | string | Opaque cursor from the board or previous page |
| `limit` | int | Default `50`, max `200` |

Response `200`:

```json
{ "items": [CaseSummary, …], "next_cursor": "opaque-or-null" }
```

---

## 9. Contact matching and case creation

### `GET /api/v1/intake/contact-matches`

Query (at least one of `email`, `phone`, `name` required):

| Parameter | Notes |
|---|---|
| `email` | Case-insensitive, trimmed |
| `phone` | Digits-only comparison, ignoring spaces and punctuation |
| `name` | Display-name or given/family match |
| `date_of_birth` | `YYYY-MM-DD`; improves name matching |
| `limit` | Default `10`, max `25` |

Response `200`:

```json
{
  "candidates": [
    {
      "id": "…",
      "display_name": "Mia Chen",
      "email": "mia@example.com",
      "phone": "0400 000 000",
      "date_of_birth": "1990-04-12",
      "lifecycle_status": "active",
      "match": "email",
      "open_case": {
        "id": "…",
        "flow_id": "…",
        "flow_name": "Disability — standard",
        "stage_name": "Risk assessment"
      }
    }
  ]
}
```

`match` is `email`, `phone`, `name_dob` or `name`. `open_case` is the most recent open case
for the candidate, if any.

### `POST /api/v1/intake/cases`

Creates a contact (or links an existing one) and a case in one transaction.

```json
{
  "client_request_id": "01J9Z8K6T7V0M2Q4R6S8T0W2X4",
  "flow_id": "…",
  "contact_id": null,
  "external_organisation_id": null,
  "contact": {
    "given_name": "Mia",
    "family_name": "Chen",
    "preferred_name": "Mia",
    "pronouns": "she/her",
    "date_of_birth": "1990-04-12",
    "gender": "female",
    "languages": ["English"],
    "cultural_identity": null,
    "email": "mia@example.com",
    "phone": "0400 000 000",
    "notes": null,
    "details": {}
  },
  "display_name": "Mia Chen",
  "source": "referral",
  "referrer_contact_id": null,
  "referrer_organisation_id": "…",
  "referral_reference": "REF-1042",
  "referral_received_at": "2026-11-26T23:10:00Z",
  "division_id": "…",
  "data": {
    "referral_reason": "Requires morning support"
  },
  "allow_duplicate_contact": false,
  "duplicate_reason": null,
  "legacy_draft_id": null
}
```

Rules:

- `client_request_id` is optional. If supplied, a retry returns the existing case instead of
  creating another. It is unique per organisation.
- `legacy_draft_id` is optional and reserved for desktop draft migration. If supplied it is
  unique per organisation and also makes the request idempotent.
- `flow_id` must be active. `subject_kind` comes from the flow.
- For person flows, supply either `contact_id` or `contact`. For organisation flows, supply
  `external_organisation_id` and omit both.
- The `contact` object accepts core identity/profile fields, not custom fields. Intake
  custom values belong in `data` as `custom:<key>` and are copied to the contact when the
  case onboards.
- `display_name` is derived from the contact when omitted. A usable name (or organisation
  name) is required.
- At most one of `referrer_contact_id` / `referrer_organisation_id` may be set; either may
  be omitted for a self-enquiry or an unrecorded referrer.
- `contact.email`/`phone` are normalised for matching.
- `data` keys must be in the field catalogue. Custom values use `custom:<key>` and are
  stored in `custom_field_values` with `entity_type = 'intake_case'`.
- `allow_duplicate_contact=false` triggers matching. If candidates exist, the request is
  refused with `409 contact_match`.
- `allow_duplicate_contact=true` requires a non-empty `duplicate_reason`; both are recorded
  in the case event.
- If the resolved contact already has an open case for this flow, the request is refused
  with `409 open_case_exists` and the open case id.
- The case starts at the flow's first stage. The first stage's checklist is materialised,
  assignment rules run, and a `created` event is written.

Response `201`:

```json
{ "case": CaseDetail, "contact": ContactSummary }
```

Duplicate contact refusal:

```json
{
  "error": {
    "code": "conflict",
    "message": "An existing contact may match this person.",
    "details": {
      "reason": "contact_match",
      "candidates": [ContactMatch, …]
    }
  }
}
```

Open case refusal:

```json
{
  "error": {
    "code": "conflict",
    "message": "This person already has an open intake for this flow.",
    "details": {
      "reason": "open_case_exists",
      "case_id": "…",
      "flow_id": "…",
      "stage_name": "Risk assessment"
    }
  }
}
```

---

## 10. Case detail and editing

### `GET /api/v1/intake/cases/{id}`

Response `200`: `CaseDetail`.

### `PATCH /api/v1/intake/cases/{id}`

```json
{
  "version": 4,
  "display_name": "Mia Chen",
  "data": {
    "risk_summary": "No known risks",
    "custom:ndis_number": "430000000"
  },
  "assignee_user_id": "…",
  "division_id": "…",
  "priority": 2,
  "force": false
}
```

- `version` is required and must match the case's current version.
- `data` is merged key-by-key; a `null` value deletes the key.
- Only keys in the current stage's `visible_fields` are writable through this endpoint.
  `force=true` requires `intake.configure` and allows correcting data from another stage;
  the event metadata records that force was used.
- `assignee_user_id` requires `intake.assign`; changing it records an `assigned` event and
  notifies the new assignee.
- `display_name` must be 1–200 characters.
- Response `200`: `CaseDetail`.

Stale version:

```json
{
  "error": {
    "code": "conflict",
    "message": "This case was changed by someone else. Reload and try again.",
    "details": {
      "reason": "version_conflict",
      "current_version": 5,
      "case": CaseDetail
    }
  }
}
```

---

## 11. Transitions

### `POST /api/v1/intake/cases/{id}/transition`

```json
{
  "to_stage_id": "…",
  "version": 4,
  "reason": null,
  "override": false,
  "override_reason": null
}
```

Transition graph in v1: any non-terminal stage can move to any other non-terminal stage or
to either terminal stage. Terminal stages have no outgoing transitions; reopening is the
explicit admin path. This supports drag-left as well as drag-right without a separate
workflow editor.

Validation order:

1. case exists in the organisation;
2. `version` matches;
3. target stage belongs to the same flow;
4. the current stage is not terminal and the target is not the current stage;
5. all current-stage `required_fields` are present and valid;
6. if `checklist_gate` is set, all required checklist items are complete;
7. if `onboarded`, the contact is valid for role linking;
8. if `cancelled`, a reason is required.

If `override=true`, steps 5 and 6 may be skipped by a user with `intake.configure` (or
`intake.override` if added later), and a reason is required.

On success:

- update `stage_id`, `stage_entered_at`, and `stage_due_at` from `sla_hours`;
- materialise the target stage's checklist template;
- resolve and assign the stage owner, writing an `assigned` event if it changes;
- apply the lifecycle nudge if legal;
- on `onboarded`, set `onboarded_at` and link the care/employment relationship;
- on `cancelled`, set `closed_at` and `cancellation_reason`;
- write a `transitioned` event and an audit entry;
- notify the assignee/watchers;
- increment `version`.

Response `200`:

```json
{ "case": CaseDetail, "event": TimelineEvent }
```

Blocked transition:

```json
{
  "error": {
    "code": "validation_failed",
    "message": "Complete the required checks before moving on.",
    "details": {
      "reason": "intake_transition_blocked",
      "missing_fields": ["date_of_birth", "custom:ndis_number"],
      "incomplete_checklist": [
        { "id": "…", "label": "Confirm identity documents", "is_required": true }
      ]
    }
  }
}
```

Illegal target:

```json
{
  "error": {
    "code": "validation_failed",
    "message": "That stage cannot be reached from the current stage.",
    "details": {
      "reason": "invalid_transition",
      "from_stage_id": "…",
      "to_stage_id": "…"
    }
  }
}
```

---

## 12. Checklist

### `POST /api/v1/intake/cases/{id}/checklist/{item_id}`

```json
{ "completed": true, "note": "Verified passport and Medicare card" }
```

- `completed=false` reopens the item and clears `completed_at`/`completed_by`.
- The note is optional; a reason is required when reopening a required item after it has
  gated a transition.
- Response `200`:

```json
{
  "item": CaseChecklistItem,
  "checklist": { "done": 3, "total": 5, "required_done": 3, "required_total": 3 }
}
```

Checklist progress is advisory until a transition checks the gate.

---

## 13. Cancellation, archive and reopen

### `POST /api/v1/intake/cases/{id}/cancel`

```json
{ "version": 4, "reason": "Client chose another provider" }
```

Sets `closed_at`, `cancellation_reason`, applies `intake_cancelled` if legal, writes a
`cancelled` event and returns `CaseDetail`. A reason is required. Cancel is valid only from
a non-terminal stage; an onboarded case is archived or handled through the contact
offboarding flow.

### `POST /api/v1/intake/cases/{id}/archive`

```json
{ "version": 4, "reason": "Onboarded and moved to the active client list" }
```

Sets `closed_at` without changing the lifecycle. Used to clear completed cards from the
Onboarded column. Requires `intake.write`. Returns `CaseDetail`.

### `POST /api/v1/intake/cases/{id}/reopen`

```json
{
  "version": 4,
  "to_stage_id": "…",
  "reason": "Onboarding was recorded in error"
}
```

Reopens a closed or onboarded case at the chosen non-terminal stage. Clears `closed_at`
and `onboarded_at`, preserves the original events, and sets the chosen stage. If the case
had onboarded a contact, the care/employment relationship created by the case is ended with
the reason `intake_reopened` and recorded in the relationship history; this also requires
`relationships.write`. Requires `intake.configure`; records a `reopened` event. If another
case for the same contact and flow is already open, the request returns `409
open_case_exists`. This is intentionally narrow and audited.

---

## 14. Timeline

### `GET /api/v1/intake/cases/{id}/timeline`

Query: `limit` (default `50`, max `200`), `before` (event id or timestamp).

Response `200`:

```json
{ "items": [TimelineEvent, …], "next": "opaque-or-null" }
```

Newest first. Field-level changes are summarised, not a full diff; sensitive values are
not echoed.

---

## 15. Waitlist

### `POST /api/v1/intake/cases/{id}/waitlist`

```json
{
  "service_id": "…",
  "division_id": "…",
  "priority": 2,
  "requested_start": "2027-02-01",
  "notes": "Prefers mornings"
}
```

Creates a waitlist entry. A case may have more than one entry (for example two services).
Response `201`: `WaitlistEntry`.

### `PATCH /api/v1/intake/waitlist/{id}`

```json
{
  "priority": 1,
  "status": "offered",
  "offer_expires_at": "2026-12-05T00:00:00Z",
  "notes": "Offer sent"
}
```

Statuses: `waiting`, `offered`, `placed`, `withdrawn`.

### `DELETE /api/v1/intake/waitlist/{id}`

Withdraws the entry (`status = withdrawn`); the row is retained. Returns `204`.

### `GET /api/v1/intake/waitlist`

Query: `division_id`, `include_descendants`, `service_id`, `status`, `priority`, `cursor`,
`limit`. Response `200`:

```json
{
  "items": [
    {
      "id": "…",
      "case": CaseSummary,
      "service": { "id": "…", "name": "Core supports" },
      "division": { "id": "…", "name": "Northern" },
      "priority": 2,
      "requested_start": "2027-02-01",
      "status": "waiting",
      "offered_at": null,
      "offer_expires_at": null,
      "notes": "Prefers mornings",
      "created_at": "…"
    }
  ],
  "next_cursor": "opaque-or-null"
}
```

---

## 16. Contact lifecycle

### `GET /api/v1/contacts/{id}/status-history`

Response `200`:

```json
{
  "items": [
    {
      "id": "…",
      "from_status": "enquiry",
      "to_status": "onboarding",
      "reason": "Eligibility confirmed",
      "actor": { "id": "…", "full_name": "Sam Lee" },
      "created_at": "2026-11-27T03:00:00Z"
    }
  ],
  "next_cursor": null
}
```

### `PATCH /api/v1/contacts/{id}/status`

```json
{
  "status": "suspended",
  "reason": "Temporary hospital admission"
}
```

Allowed transitions:

| From | To |
|---|---|
| `enquiry` | `onboarding`, `active`, `intake_cancelled` |
| `onboarding` | `active`, `intake_cancelled` |
| `active` | `suspended`, `offboarding`, `offboarded` |
| `suspended` | `active`, `offboarding`, `offboarded` |
| `offboarding` | `offboarded`, `active` |
| `intake_cancelled` | `enquiry` |
| `offboarded` | `enquiry` |

A reason is required for `suspended`, `offboarding`, `offboarded` and `intake_cancelled`.
Illegal transitions return `422 lifecycle_transition_invalid`. Requires `contacts.write`.
A new contact starts with an initial `null -> enquiry` history entry. Response `200`: the
updated contact summary plus the new history entry.

Statuses are independent of intake and may be extended later. Stage nudges use the same
transition rules. They never overwrite `suspended`, `offboarding` or `offboarded`, and a
cancelled stage never downgrades an `active` contact. `onboarded` may move
`enquiry`/`onboarding` to `active`; `cancelled` may move `enquiry`/`onboarding` to
`intake_cancelled`. Any other nudge is skipped and logged.

---

## 17. Notifications

### `GET /api/v1/notifications`

Query: `unread` (bool), `limit` (default `30`, max `100`), `before` (id). Response `200`:

```json
{
  "items": [Notification, …],
  "unread_count": 3,
  "next_cursor": "opaque-or-null"
}
```

### `POST /api/v1/notifications/{id}/read`

Marks one notification read. Returns `204`. Reading another user's notification is `404`.

### `POST /api/v1/notifications/read-all`

Marks every unread notification for the caller read. Returns:

```json
{ "unread_count": 0 }
```

Notification creation is server-side. A user does not receive a notification for their own
action. Retries do not duplicate notifications; the dedupe key is
`case:<case_id>:stage:<stage_id>:<event_id>`.

---

## 18. Action policy enforcement

The stage's `actions` map is enforced in addition to capabilities.

| Action key | Guarded endpoints |
|---|---|
| `profile.edit` | `PATCH /contacts/{id}`, contact custom-field writes |
| `relationships.update` | Create, correct, end, transfer, change-role, remove relationship |
| `funding.add` | Create funding package, create agreement, add line, finalise |
| `funding.edit` | Update package/agreement status, edit line |
| `funding.remove` | Delete agreement line, remove package where supported |
| `roster.add` | Create shift/batch with the contact, create assignment |
| `roster.remove` | Update/delete assignment for the contact |

Policy resolution:

- Find open cases for every contact targeted by the action.
- If no open case exists, the action is allowed by the capability alone.
- If one or more open cases exist, the action is allowed only when every case's current
  stage allows it.
- A case is "open" when `closed_at IS NULL AND onboarded_at IS NULL`.
- The most restrictive denial wins.

Refusal:

```json
{
  "error": {
    "code": "conflict",
    "message": "Funding changes are locked during risk assessment.",
    "details": {
      "reason": "intake_action_blocked",
      "action": "funding.add",
      "contact_id": "…",
      "case_id": "…",
      "flow_id": "…",
      "stage_id": "…",
      "stage_name": "Risk assessment"
    }
  }
}
```

Contact detail responses add an `intake` block so clients can disable controls proactively:

```json
{
  "…": "existing contact detail fields",
  "intake": {
    "case_id": "…",
    "flow_id": "…",
    "flow_name": "Disability — standard",
    "stage_id": "…",
    "stage_name": "Risk assessment",
    "lifecycle_status": "onboarding",
    "allowed_actions": ["profile.edit", "relationships.update"],
    "denied_actions": {
      "funding.add": "Funding changes are locked during risk assessment."
    }
  }
}
```

If several cases apply, `allowed_actions` is the intersection and `denied_actions` explains
each denial. When no open case exists, `intake` is `null` and all capability-allowed
actions are available.

---

## 19. Field catalogue and validation

- Built-in keys are the stable keys used by the existing wizard (`first_name`, `last_name`,
  `email`, `phone`, `division`, …). They map to contact columns or contact `details`.
- Custom fields are addressed as `custom:<key>` and resolve to the organisation's active
  `contact` custom-field definitions.
- `GET /intake/field-catalog` is authoritative for labels, types, options and sections.
- `visible_fields` controls what the case detail shows for the current stage.
- `required_fields` controls what must be present before leaving the stage.
- Values are validated by the same rules as contact custom fields: type, options,
  min/max length, numeric range and regex.
- A field may be visible in several stages; earlier values remain in the "captured so far"
  summary even when the current stage does not list them.
- Onboarding copies built-in values to contact columns/`details` and custom values to the
  contact's custom-field values. The copy is idempotent and only writes keys present in the
  case; values already on the contact but absent from the case are left untouched.

### Expected built-in keys

The server catalogue should initially mirror the desktop wizard and the contact model. At
minimum:

`first_name`, `last_name`, `preferred_name`, `pronouns`, `date_of_birth`, `gender`,
`languages`, `cultural_identity`, `email`, `phone`, `address_line1`, `address_line2`,
`suburb`, `state`, `postcode`, `division`, `occupation`, `employer`, `ndis_number`,
`medicare_number`, `health_summary`, `support_needs`, `risk_summary`, `behaviour_notes`,
`consent_services`, `consent_share`, `emergency_contact_name`, `emergency_contact_phone`,
`gp_name`, `gp_phone`, `position`, `employment_type`, `start_date`, `award`,
`classification`, `pay_point`, `superannuation`, `notes`.

The exact list is fixed in `crates/server/src/intake/fields.rs`; the desktop compatibility
test prevents drift while the old wizard exists.

The legacy `division` field is free text from the contact profile. It is **not** the same as
`intake_cases.division_id`; the latter is the authoritative division for filtering and
reporting. Onboarding may copy the text for display, but it must not be used for access or
board filters.

---

## 20. Division filtering

- `division_id` selects a division.
- `include_descendants=true` (default) includes every child division recursively. This is
  how a parent-division filter includes subdivisions.
- `include_descendants=false` matches the selected division exactly.
- A flow may be scoped to a division. The flow picker shows flows whose division is the
  selected division or an ancestor, plus unscoped flows.
- Division filtering is not authorisation. A user who can read intake can read every
  division's cases; the filter narrows the view only.

---

## 21. Concurrency and idempotency

- Every case has an integer `version`, incremented on every write.
- `PATCH` and `POST transition/cancel/archive/reopen` require the current `version`.
- A stale version returns `409 version_conflict` with the current case.
- `client_request_id` on case creation makes retries idempotent per organisation.
- `legacy_draft_id` makes migration from desktop drafts idempotent.
- Notification dedupe keys make assignment retries safe.
- Transition is atomic: stage change, checklist materialisation, assignment, lifecycle
  nudge, role link, event, audit and notification either all commit or none do.

---

## 22. Seed configuration

Each new organisation receives eight default flows:

| Subject kind | Variant | Parent | Notes |
|---|---|---|---|
| `client` | `standard` | — | Default client flow |
| `client` | `aged_care` | — | Aged-care-specific fields |
| `client` | `disability_standard` | — | Disability standard |
| `client` | `disability_emergency` | — | Disability emergency |
| `client` | `fee_for_service` | — | Private/fee-for-service |
| `worker` | `standard` | — | Worker onboarding |
| `other_contact` | `standard` | — | Coordinators, family, GPs |
| `organisation` | `standard` | — | External organisation intake |

Flow keys must be unique; the seed uses keys such as `client_standard`,
`client_aged_care`, `client_disability_standard`,
`client_disability_emergency`, `client_fee_for_service`, `worker_standard`,
`other_contact_standard`, `organisation_standard`.

The generic `client_disability` parent flow was removed after discovery: it
duplicated the standard pipeline, and the two disability variants are clearer as
top-level flows. Parent flows remain supported for organisations that need them.

Every flow starts with six stages:

| Key | Kind | Terminal | Default lifecycle nudge |
|---|---|---|---|
| `enquiry` | `enquiry` | no | `enquiry` |
| `eligibility` | `eligibility` | no | `onboarding` |
| `risk_assessment` | `risk_assessment` | no | `onboarding` |
| `service_planning` | `service_planning` | no | `onboarding` |
| `onboarded` | `onboarded` | yes | `active` |
| `cancelled` | `cancelled` | yes | `intake_cancelled` |

Suggested default action policy:

| Stage | profile.edit | relationships.update | funding.add/edit/remove | roster.add/remove |
|---|:---:|:---:|:---:|:---:|
| Enquiry | no | no | no | no |
| Eligibility | yes | yes | no | no |
| Risk assessment | yes | yes | no | no |
| Service planning | yes | yes | add/edit | no |
| Onboarded | yes | yes | yes | yes |
| Cancelled | no | no | no | no |

Suggested default checklist:

| Stage | Items |
|---|---|
| Enquiry | Record source; confirm contact details |
| Eligibility | Confirm identity; confirm funding/eligibility; record consent |
| Risk assessment | Complete risk screen; record behaviour support needs; assign risk owner |
| Service planning | Confirm services; confirm funding; confirm schedule preferences; service agreement |
| Onboarded | Verify care/employment role and funding are active; confirm first shift plan |
| Cancelled | Record cancellation reason; notify referrer (optional); archive documents |

All defaults are editable in `/admin/intake`. Required fields, checklists and assignees are
seeded conservatively and should be reviewed before enabling the board.

---

## 23. Admin UI

`/admin/intake` provides:

- flow list grouped by subject kind and parent;
- flow create/clone/archive;
- stage list with drag reorder, label, colour, kind, terminal badge, lifecycle nudge, SLA
  and checklist gate;
- field visibility and required-field multi-selects grouped by catalogue section;
- action policy toggles;
- checklist template editor with required flags;
- assignment rules: named user, permission-set pool, strategy, notify, order;
- a preview of the resulting column and field layout.

All write forms require the existing CSRF double-submit token and `intake.configure`.
Changes are written to `audit_log`.

---

## 24. Change policy

During implementation, every endpoint and DTO in this contract must be annotated with
`utoipa` and appear in `/api-docs/openapi.json` / `/docs`. The markdown contract and the
generated OpenAPI document must agree; CI should fail if a route is missing from the spec.

- This contract is additive within `/api/v1`. New optional request fields and response
  fields are non-breaking; clients must ignore unknown fields.
- Removing or changing the meaning of a field requires `/api/v2`.
- Stable enums (`subject_kind`, `stage_kind`, lifecycle statuses, action keys, error
  reasons) are part of the contract. Adding a value is a compatible change for readers that
  tolerate unknown values; clients should default gracefully.
- The field catalogue keys are stable. Custom field keys are already governed by the
  custom-fields contract and must not be renamed.
