# CareLibre Server

The server for **CareLibre**, an open-source care management platform for
Australian disability (NDIS) and aged care providers.

This repository contains the backend only: a REST API, PostgreSQL schema and
deployment tooling. The desktop (Rust/egui), Android and iOS clients live in
separate repositories and talk to this server over HTTP.

> **Status: early development.** Authentication, multi-tenancy, the care model,
> relationships, rostering APIs and the admin tooling are in place. Native
> clients live in separate repositories. This is a one-maintainer project: the
> interfaces are usable, but expect them to change as the design settles. See
> [`docs/roadmap.md`](docs/roadmap.md) for what is done and what is next.

## What CareLibre is trying to do differently

CareLibre is not trying to match every feature of a mature commercial care
suite. These are the design choices it is built around.

- **People are people; roles are relationships.** Contacts are untyped. Client,
  worker, guardian, GP and emergency contact are roles on a relationship, so one
  person can hold several roles without duplicate records. Each relationship
  stores a role for both endpoints, so a doctor/patient link reads correctly from
  either side.

- **Rostering suggestions show their working.** The suitability endpoint ranks
  workers from shift history and explicit constraints, and returns the reasons
  and warnings behind every score. It recommends; it never auto-assigns.

- **Preferences and conflicts are explicit.** `preferred`, `avoid` and `blocked`
  constraints can target a client, another worker, a site, a service or a
  division. Hard blocks stay hard; softer preferences act as bonuses or
  penalties.

- **History accumulates; scores are recomputed.** Assignment outcomes and
  attendance feed history views. Care relationships are not overwritten by
  learned affinity, so recommendations can be explained, audited or replaced
  later as the model improves.

- **Self-hosted by default.** The server is a single Rust binary backed by
  PostgreSQL, with the admin UI, Swagger UI and migrations included. The
  Community Edition runs with Docker Compose, and data stays in the provider's
  own database.

- **API-first.** The desktop, Android and iOS clients use the same OpenAPI
  surface. The web admin is limited to server and organisation settings, not
  day-to-day care work.

## Stack

| Concern | Choice | Rationale |
|---|---|---|
| Language | Rust (stable) | Same language as the planned native clients |
| HTTP | axum 0.8 | Tokio-native, minimal, excellent extractor model |
| Database | PostgreSQL 16+ only | JSONB, range types, RLS for later multi-tenancy |
| Data access | sqlx (runtime-checked) | Async, no ORM lock-in |
| Auth | Argon2id + HS256 JWT + opaque rotating refresh tokens | |
| API docs | OpenAPI 3 via utoipa, Swagger UI | SDK generation for the mobile/desktop clients |
| Deploy | Docker Compose | Self-hosted Community Edition must be one command |

See [`docs/adr/`](docs/adr/) for the decisions and their trade-offs.
The domain schema is described in [`docs/data-model.md`](docs/data-model.md).
A repeatable demo dataset is described in [`docs/demo-data.md`](docs/demo-data.md)
(`make seed-demo`).

Connecting the desktop/mobile clients: [`docs/connecting-clients.md`](docs/connecting-clients.md).

## Quick start (Docker)

```sh
cp .env.example .env
# edit .env and set a real JWT_SECRET: openssl rand -hex 32
docker compose up --build
```

- Status page: <http://localhost:8484/>
- Admin UI: <http://localhost:8484/admin>
- API: <http://localhost:8484/api/v1>
- Swagger UI: <http://localhost:8484/docs>
- OpenAPI JSON: <http://localhost:8484/api-docs/openapi.json>
- Health: `GET /healthz`, `GET /readyz`

Migrations run automatically at startup (`RUN_MIGRATIONS=true`).

## Quick start (local)

Requires PostgreSQL 16+ and a Rust toolchain.

```sh
make setup                      # copies .env.example to .env
createdb carelibre              # or use the compose PostgreSQL
make run
```

## Configuration

All configuration is via environment variables (see `.env.example`).

| Variable | Default | Notes |
|---|---|---|
| `APP_ENV` | `development` | `production` enables JSON logs and strict validation |
| `BIND_ADDR` | `0.0.0.0:8484` | |
| `DATABASE_URL` | — | Required |
| `DB_MAX_CONNECTIONS` | `10` | |
| `RUN_MIGRATIONS` | `true` | |
| `JWT_SECRET` | generated in dev | **Required** in production, ≥ 32 chars |
| `ACCESS_TOKEN_TTL_SECS` | `900` | |
| `REFRESH_TOKEN_TTL_SECS` | `2592000` | 30 days |
| `CORS_ALLOWED_ORIGINS` | empty | Comma-separated; `*` rejected in production |

## API

The full spec is served at `/api-docs/openapi.json` and rendered at `/docs`.
Application routes live under `/api/v1`:

| Area | Endpoints |
|---|---|
| Auth | `register`, `login`, `refresh`, `logout`, `me` |
| People | contacts and their custom fields |
| Relationships | list, create, delete and the relationship graph |
| Rostering | sites, shifts, assignments, explicit constraints and worker suitability |
| Organisation | profile, name and logo upload/removal |
| Health | `GET /healthz`, `GET /readyz` |

Every error uses one envelope:

```json
{ "error": { "code": "validation_failed", "message": "password must be at least 12 characters" } }
```

Example:

```sh
curl -sX POST localhost:8484/api/v1/auth/register \
  -H 'content-type: application/json' \
  -d '{
    "organisation_name": "Example Care",
    "organisation_slug": "example-care",
    "full_name": "Jane Doe",
    "email": "jane@example.com",
    "password": "correct horse battery staple"
  }'
```

## Layout

```
crates/server/src/
  admin/        server-rendered admin UI, network graph, org settings
  api/          HTTP routes, DTOs, OpenAPI
  auth/         password hashing, JWTs, extractor, roles
  config.rs     environment configuration
  db.rs         pool + migrations
  error.rs      error envelope
  organisation.rs  profile and logo branding
  relationships.rs two-sided relationship graph
  rostering.rs  suitability scoring
  state.rs      shared app state
migrations/     sqlx migrations (forward-only)
docs/           architecture, ADRs, admin, roadmap
```

## Administration

`/admin` is a server-rendered admin area for owners and administrators:

- dashboard with user/session counts and recent audit events
- user and active-session lists
- database explorer mapping tables, columns, keys and relationships (structure
  only — no record browsing)
- relationship network with whole-organisation and per-party views
- organisation settings: display name and logo used by clients
- custom fields for every object, with types, validation rules, archive and a
  guarded delete (see [`docs/custom-fields.md`](docs/custom-fields.md))
- capability-based permission sets with seven OOTB personas and per-user
  assignment (see [`docs/permissions.md`](docs/permissions.md))
- one-time invitation links carrying permission sets (see
  [`docs/invitations.md`](docs/invitations.md))
- CSV import with templates, dry-run validation and all-or-nothing commit
  (see [`docs/import.md`](docs/import.md))
- onboarding checklist with automatic task detection at `/admin/onboarding`
  (see [`docs/onboarding.md`](docs/onboarding.md))

See [`docs/admin.md`](docs/admin.md) for access rules and security notes.
The planned first-run experience is described in
[`docs/onboarding.md`](docs/onboarding.md); the authorisation model is in
[`docs/permissions.md`](docs/permissions.md).

## Development
make lint     # clippy, warnings as errors
make fmt      # rustfmt
make build
```

CI runs fmt, clippy, tests and migration validation against a live PostgreSQL
service, plus a Docker image build.

## License

AGPL-3.0-or-later. See [`LICENSE`](LICENSE).
