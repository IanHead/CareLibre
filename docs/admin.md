# Administration

Server-rendered admin UI at **`/admin`**. It is deliberately dependency-free —
plain HTML and inline CSS, no client-side framework and no JavaScript.

## Access

- Sign in at `/admin/login` with an **owner** or **admin** account.
- Accounts with `manager`, `staff` or `viewer` roles are rejected (HTTP 403).
- If an account belongs to several organisations, supply the organisation slug
  at sign-in; otherwise the sole organisation is used.

The session is an opaque token (the same kind as an API refresh token) stored in
a `carelibre_admin` cookie:

| Attribute | Value |
|---|---|
| `HttpOnly` | yes — not readable from JavaScript |
| `SameSite` | `Lax` |
| `Secure` | yes when `APP_ENV=production` |
| Lifetime | browser session; server-side expiry follows `REFRESH_TOKEN_TTL_SECS` |

Signing out revokes the row in `auth_sessions`, so the token is dead even if the
cookie is replayed.

## Pages

| Path | Contents |
|---|---|
| `/admin` | Organisation overview: member and session counts, role, uptime, recent audit events |
| `/admin/users` | Login accounts for the current organisation: name, email, role, status, last login |
| `/admin/sessions` | Live sessions with client user-agent, start and expiry; marks the current session |
| `/admin/schema` | Database explorer (see below) |

All pages are organisation-scoped: queries bind `organisation_id` from the
resolved session, never from the URL.

## Database explorer

`/admin/schema` maps the **data model**, not the data:

- an SVG entity-relationship diagram with primary-key and foreign-key markers
- every table with its columns, types, nullability, defaults and keys
- indexes, including unique flags
- a relationship list (`table.column → target.column`)

It reads PostgreSQL catalog views only (`pg_class`, `information_schema.*`,
`pg_indexes`). There is no record browser and no query endpoint, by design. The
`_sqlx_migrations` bookkeeping table is hidden.

## Security notes

- Role gating happens on every request, not just at sign-in.
- The session token is stored hashed (SHA-256) in `auth_sessions`.
- All user-controlled values are HTML-escaped before rendering.
- Database errors are logged server-side and replaced with a generic message —
  driver errors can contain the connection string.
- **CSRF:** state-changing endpoints (`POST /admin/login`, `POST /admin/logout`)
  rely on `SameSite=Lax`. Before adding destructive admin actions, introduce
  per-form CSRF tokens.
- There is no rate limiting on the admin sign-in form yet.
