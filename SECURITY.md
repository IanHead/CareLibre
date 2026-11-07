# Security policy

## Reporting a vulnerability

Do **not** open a public issue. Email the maintainers privately, or use GitHub's
private vulnerability reporting on this repository. Include:

- affected version/commit
- reproduction steps
- impact assessment

We aim to acknowledge within 3 business days and to ship a fix or mitigation
within 30 days for high-severity issues.

## Scope

This repository is the CareLibre server. It handles personal and health-adjacent
information about care recipients and workers, so the following are always in
scope:

- authentication and session handling
- tenant isolation (cross-organisation data access)
- authorisation bypass
- injection (SQL, template, command)
- secrets exposure
- insecure defaults in the Docker deployment

## Deployment expectations

- `APP_ENV=production` requires an explicit `JWT_SECRET` of at least 32 chars.
- Serve the API over TLS. The server does not terminate TLS itself; place it
  behind a reverse proxy or a managed ingress.
- Rotate `JWT_SECRET` by deploying a new value during a maintenance window;
  existing access tokens are invalidated (refresh tokens remain valid).
- Restrict database access to the application network.

## Known gaps (tracked)

- Refresh-token reuse detection (revoking the session family) is not yet
  implemented.
- PostgreSQL row-level security is planned as defence in depth; isolation is
  currently enforced in application code only.
- No rate limiting on `/auth/login`, `/auth/register` or the admin sign-in form
  yet.
- Custom-field mutations use per-form CSRF tokens. Login, logout and other
  low-risk forms still rely on `SameSite=Lax`.
