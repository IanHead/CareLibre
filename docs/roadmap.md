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
- Onboarding checklist and dashboard (see [`onboarding.md`](onboarding.md))
- One-time invitation links and CSV import with templates
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

- **Shifts** — client/worker assignments, sessions, open-hours blocks
- **Client assignments**, **worker assignments**, **sessions**
- Copy/paste shifts across days/weeks/workers
- Conflict detection, travel time, availability
- Social graph for worker ↔ client matching

## Phase 4 — Money and integrations

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
