# Administration

Server-rendered admin UI at **`/admin`**. It is deliberately dependency-free —
plain HTML and inline CSS, no client-side framework and no JavaScript.

## Access

- Any member of the organisation may sign in; what they see is determined by
  their **permission sets** (capabilities), never by a role name.
- Owners bypass capability checks. Everyone else needs the capability for a page
  (for example `users.manage` for Users, `schema.view` for the database explorer).
- Navigation items the account cannot use are hidden, and the handler returns
  `403` if the URL is requested directly.
- Accounts with no permission sets can sign in but see only a bare dashboard.
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
| `/admin` | Organisation overview: user and session counts, access label, uptime; recent audit events when permitted |
| `/admin/users` | Login accounts, effective permission sets and access editing (`users.manage`) |
| `/admin/sessions` | Live sessions with client user-agent, start and expiry (`users.manage` or `audit.read`) |
| `/admin/permission-sets` | Capabilities and the seven OOTB personas (`permission_sets.manage`) |
| `/admin/schema` | Database explorer (see below) |
| `/admin/custom-fields` | User-defined fields for every object (`custom_fields.manage`) |
| `/admin/users/{id}/access` | Assign one or more permission sets to a user |
| `/admin/invitations` | Create, inspect and revoke one-time invitation links (`users.manage`) |

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

- Capability checks happen on every request, not just at sign-in. Owners bypass them.
- The session token is stored hashed (SHA-256) in `auth_sessions`.
- All user-controlled values are HTML-escaped before rendering.
- Database errors are logged server-side and replaced with a generic message —
  driver errors can contain the connection string.
- **CSRF:** custom-field and permission-set mutations use double-submit tokens
  (HttpOnly cookie plus a hidden form field). Login/logout still rely on
  `SameSite=Lax`.
- The Administrator set is immutable and the last owner cannot be demoted — both
  enforced by database triggers, not just the UI.
- There is no rate limiting on the admin sign-in form yet.
