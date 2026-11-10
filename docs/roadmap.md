# Roadmap

Derived from the CareLibre project brief. The server is built in vertical
slices: each phase ships schema, API, tests and docs together.

## Phase 0 — Foundation (this milestone) ✅

- Cargo workspace, modular crate layout
- Configuration from environment, secrets hygiene
- PostgreSQL pool + forward-only migrations
- Organisations (tenants), users, memberships, roles
- Argon2id passwords, JWT access tokens, rotating refresh tokens
- Uniform error envelope, audit log skeleton
- OpenAPI + Swagger UI, health/readiness probes
- Dockerfile, compose stack, CI (fmt/clippy/test/migrations/image)

## Phase 1 — Operational basics

- Invite flow, password reset, email verification
- Account settings, organisation settings
- Capability-based permission sets and OOTB personas (Phase 1 done)
- Pagination, filtering and sorting conventions
- Request IDs and structured audit for every mutation
- Onboarding checklist and dashboard (done)
- One-time invitation links and CSV import with templates (both done)
- Organisation name, logo and client branding (done)
- PostgreSQL row-level security across tenants
- Metrics endpoint (Prometheus)

## Phase 2 — Core care objects

- **Divisions** — organisational units
- **Cost centres** — hierarchical, with configurable inheritance paths
- **Contacts** — untyped people record; client/workers/doctors/family are roles
- **External organisations** — providers, brokers, health services
- **Relationships** — typed links between contacts and organisations
- **Services** — mapped to NDIS/Aged Care funding codes
- **Rates** and **rate versions** (effective-dated pricing)
- **Rostering schema** — shifts, sessions, client/worker assignments
- Custom fields: definitions, types and validation (`/admin/custom-fields`);
  form layout editor still to come
- Document merge templates
- REST API and validation for the above

## Phase 3 — Rostering

- **Shifts** — client/worker assignments, sessions, open-hours blocks ✅
- **Sites, services and shift context** ✅
- **Roster constraints** — preferred, avoid and hard blocked links ✅
- **Assignment outcomes and ratings** ✅
- **Explainable worker suitability endpoint** ✅
- Conflict detection, travel time, availability
- Copy/paste shifts across days/weeks/workers
- Social graph / learned ranking for worker ↔ client matching

### Worker–client affinity (next)

- **Client preferences about workers** — record preferences at intake, for
  example “older worker”, “bubbly/talkative”, “quiet”, “female”, or “same
  cultural background”. These are the client’s preferences, not worker traits:
  workers are never asked to declare personality, age or similar attributes.
- **Learned affinity, not declarations** — when a worker works repeated
  shifts with clients who hold a preference, that history becomes evidence.
  The scorer explains it as, for example, “6 attended shifts with clients who
  prefer a bubbly, talkative worker”.
- **Relationship-aware recommendations** — also consider whether the worker has
  successfully supported other clients connected to the current client
  (family, housemates, shared support coordinator, same site or service). This
  reuses the existing relationship graph and assignment history rather than
  adding a second social model.
- **Advisory and explainable** — preference and network evidence adjusts ranking
  and adds reasons or warnings; it never hard-filters unless an explicit block
  is recorded. The coordinator remains the decision-maker.
- **Sensitive preferences** — age, gender, cultural background and personality
  descriptors are sensitive. Require a reason or legal basis, keep the evidence
  aggregate rather than exposing an inferred worker trait, and avoid
  discriminatory hard constraints.
- **Schema sketch** — `client_worker_preferences` (client, dimension, value,
  strength, reason), with derived views such as `worker_preference_affinity`
  and `worker_client_network_affinity`. No worker trait columns.
- **Scoring** — extend `ScoreInput` with `PreferenceEvidence` and
  `NetworkAffinityEvidence`, and surface reasons such as “previously worked
  with three clients connected to this client”.

## Phase 4 — Money and integrations

- **Funding sources** — NDIS, Support at Home, CHSP, private and arbitrary sources with a claiming model ✅
- **Client funding packages** — plan/package references, dates, budgets and utilisation ✅
- **Quotes and service agreements** — line items, rates, recurring schedules and finalise-to-roster generation ✅
- Claims submission and remittance for NDIS, Support at Home and CHSP
- Price-guide imports and plan budget alerts
- Natural-language quoting
- Invoicing and claims (NDIS/Aged Care)
- Xero, QuickBooks, Salesforce, Power BI connectors
- Revenue reporting

## Phase 5 — Communications and intake

- Configurable external communications (SMS, email) with provider adapters
- Wizard-style intake processes
- Notification templates and scheduling

## Phase 6 — Editions and operations

- Edition gating (`community` / `business` / `enterprise`)
- Licensing, support entitlements
- Backup/restore tooling, upgrade path
- Hardening: rate limiting, refresh-token reuse detection, SSO

## Explicitly out of scope for this repo

- Desktop, Android and iOS clients (separate repositories)
- Marketing site and billing infrastructure
