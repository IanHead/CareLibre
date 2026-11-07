# CareLibre Server

The server for **CareLibre**, an open-source care management platform for
Australian disability (NDIS) and aged care providers.

This repository contains the backend only: a REST API, PostgreSQL schema and
deployment tooling. The desktop (Rust/egui), Android and iOS clients live in
separate repositories and talk to this server over HTTP.

> **Status: foundation.** Authentication, multi-tenancy and the migration
> pipeline are in place. Care-domain objects (divisions, cost centres, contacts,
> services, rates, rostering) are next — see [`docs/roadmap.md`](docs/roadmap.md).

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

| Method | Path | Auth | Description |
|---|---|---|---|
| `GET` | `/healthz` | — | Liveness |
| `GET` | `/readyz` | — | Readiness (checks PostgreSQL) |
| `POST` | `/api/v1/auth/register` | — | Create organisation + owner account |
| `POST` | `/api/v1/auth/login` | — | Exchange credentials for tokens |
| `POST` | `/api/v1/auth/refresh` | — | Rotate refresh token |
| `POST` | `/api/v1/auth/logout` | — | Revoke refresh token |
| `GET` | `/api/v1/me` | Bearer | Current user + organisation |

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
  admin/        server-rendered admin UI and schema explorer
  api/          HTTP routes, DTOs, OpenAPI
  auth/         password hashing, JWTs, extractor, roles
  config.rs     environment configuration
  db.rs         pool + migrations
  error.rs      error envelope
  state.rs      shared app state
migrations/     sqlx migrations (forward-only)
docs/           architecture, ADRs, admin, roadmap
```

## Administration

`/admin` is a server-rendered admin area for owners and administrators:

- dashboard with member/session counts and recent audit events
- member and active-session lists
- database explorer mapping tables, columns, keys and relationships (structure
  only — no record browsing)

See [`docs/admin.md`](docs/admin.md) for access rules and security notes.

## Development
make lint     # clippy, warnings as errors
make fmt      # rustfmt
make build
```

CI runs fmt, clippy, tests and migration validation against a live PostgreSQL
service, plus a Docker image build.

## License

AGPL-3.0-or-later. See [`LICENSE`](LICENSE).
