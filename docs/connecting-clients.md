# Connecting a client

Everything a desktop, Android or iOS client needs to talk to the server.

## Base URL

| Deployment | Base URL |
|---|---|
| Local development | `http://127.0.0.1:8484` |
| Another machine on the LAN | `http://<server-lan-ip>:8484` |
| Production | `https://<your-domain>` (TLS terminated by a reverse proxy) |

All application endpoints live under **`/api/v1`**. The prefix is part of the
contract; a future breaking change becomes `/api/v2`.

| Port | Purpose |
|---|---|
| `8484` | HTTP API (configurable via `BIND_ADDR`) |
| `5432` | PostgreSQL in the compose stack — **do not expose to clients** |
| `55432` | PostgreSQL for the local bundled development cluster |

## Discovery

| Endpoint | Returns |
|---|---|
| `GET /` | human-readable status page with live database check and connection guide |
| `GET /healthz` | process liveness |
| `GET /readyz` | readiness, including database connectivity |
| `GET /api-docs/openapi.json` | OpenAPI 3 document — generate client bindings from this |
| `GET /docs` | Swagger UI for manual exploration |

Regenerate a client when the spec changes rather than hand-writing DTOs.

## Authentication flow

1. `POST /api/v1/auth/register` — creates an organisation and its `owner`
   account, and returns a token pair. Use once, at first-run setup.
2. `POST /api/v1/auth/login` — email + password, optional `organisation_slug`
   when the account belongs to more than one organisation.
3. Send `Authorization: Bearer <access_token>` on every request.
4. When a call returns `401`, refresh with `POST /api/v1/auth/refresh` and retry
   **once**. Refresh tokens are single-use: the response contains a new pair,
   and the old refresh token is now dead.
5. `POST /api/v1/auth/logout` revokes the current refresh token.

| Token | Lifetime | Client storage |
|---|---|---|
| Access (JWT) | 15 min (`ACCESS_TOKEN_TTL_SECS`) | in memory preferred |
| Refresh (opaque) | 30 days (`REFRESH_TOKEN_TTL_SECS`) | OS keychain / Keystore / Keychain |

Persist the refresh token, not the access token. Never log either.

### Example

```sh
# Login
curl -sX POST http://127.0.0.1:8484/api/v1/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"jane@example.com","password":"correct horse battery staple",
       "organisation_slug":"example-care"}'

# Authenticated call
curl -s http://127.0.0.1:8484/api/v1/me \
  -H "authorization: Bearer $ACCESS_TOKEN"

# Refresh (old refresh token is invalidated)
curl -sX POST http://127.0.0.1:8484/api/v1/auth/refresh \
  -H 'content-type: application/json' \
  -d '{"refresh_token":"..."}'
```

## Errors

Every non-2xx response has the same shape:

```json
{ "error": { "code": "validation_failed", "message": "...", "details": {} } }
```

`code` is stable and safe to branch on; `message` is user-presentable. Treat
`5xx` as retryable with backoff; `4xx` is a client bug.

## CORS

Native clients (Rust/egui, Android, iOS) are not browsers and are unaffected by
CORS. If a client embeds a web view, set `CORS_ALLOWED_ORIGINS` to the exact
origins — `*` is rejected in production.

## Production checklist

- `APP_ENV=production` and a unique `JWT_SECRET` of at least 32 characters.
- Terminate TLS in front of the server; the binary speaks plain HTTP.
- Do not publish the PostgreSQL port.
- Point `DATABASE_URL` at a managed or backed-up PostgreSQL instance.
- Set `RUN_MIGRATIONS=true` on one instance only, or run migrations as a
  separate deploy step if you scale horizontally.
