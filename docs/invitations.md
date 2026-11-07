# Invitations

One-time shareable links for adding users. No email delivery is required for the
server to be useful: the administrator copies the link and sends it through
whatever channel they already trust.

## Implementation status

Implemented:

- `/admin/invitations` list, create and revoke (`users.manage`);
- public `/invite/<token>` acceptance with password creation and auto sign-in;
- SHA-256 hashed tokens, single-use, expiry, revocation and audit entries;
- permission sets applied at acceptance, and inviters cannot grant capabilities
  they do not hold (owners exempt);
- `Cache-Control: no-store` and `Referrer-Policy: no-referrer` on invite pages.

Not yet implemented:

- email delivery and reminders;
- accepting as an existing signed-in account;
- rate limiting per IP/token;
- REST API endpoints (the admin UI uses server-rendered routes).

## Why links rather than email

- Self-hosted Community Edition often has no SMTP configured; requiring email
  would block the first team member.
- The link is the secret. It is shown once, works once, and expires.
- Email can be added later as a delivery mechanism for the same token; the
  acceptance flow does not change.

## Flow

```
Admin: Users → Invite
  ├── enter email, full name, permission sets, expiry (default 7 days)
  ├── server creates invitation + token hash
  └── link displayed exactly once: /invite/<token>
        │
        ▼
Invitee opens link
  ├── GET validates token (exists, not accepted, not revoked, not expired)
  ├── form: email (read-only), name, password, confirm
  └── POST accepts:
        ├── re-validates token
        ├── creates user (new accounts only, v1)
        ├── creates organisation_users row (is_owner = false)
        ├── assigns the invitation's permission sets
        ├── marks accepted, writes audit
        └── signs them in and redirects to /admin
```

## Token rules

| Property | Value |
|---|---|
| Entropy | 32 random bytes, base64url (43 chars) |
| Storage | SHA-256 hash only; the plaintext is never persisted |
| Lookup | Constant-time by indexed hash |
| Lifetime | Default 7 days; allowed 1–30 |
| Use | Single-use; `accepted_at` set on success |
| Revocation | `revoked_at`; administrator can revoke a pending invite |
| Display | Shown once at creation; if lost, revoke and reissue |
| Logging | Never log the token or the full URL |
| Response headers | `Referrer-Policy: no-referrer`, `Cache-Control: no-store` |
| Rate limiting | Per-IP and per-token attempts (planned) |

## Data model

```sql
CREATE TABLE invitations (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    email             text NOT NULL CHECK (char_length(email) <= 320),
    full_name         text,
    token_hash        text NOT NULL,
    expires_at        timestamptz NOT NULL,
    accepted_at       timestamptz,
    accepted_user_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    revoked_at        timestamptz,
    created_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT invitations_token_key UNIQUE (token_hash),
    CONSTRAINT invitations_expiry CHECK (expires_at > created_at)
);

-- One pending invitation per email per organisation.
CREATE UNIQUE INDEX invitations_pending_key
    ON invitations (organisation_id, lower(email))
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE TABLE invitation_permission_sets (
    organisation_id   uuid NOT NULL,
    invitation_id     uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    PRIMARY KEY (invitation_id, permission_set_id),
    FOREIGN KEY (organisation_id, invitation_id)
        REFERENCES invitations (organisation_id, id) ON DELETE CASCADE,
    FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE RESTRICT
);
```

`invitations` needs `UNIQUE (organisation_id, id)` for the composite FK, matching
the tenancy pattern used elsewhere.

## Acceptance behaviour

### New accounts (v1)

1. Validate the token.
2. Reject if the email already belongs to a user account: "This email already has
   an account. Sign in, then ask an administrator to add you to the organisation."
   This prevents an invitation from resetting an existing password.
3. Enforce the same password policy as registration (minimum 12 characters).
4. Create the user, membership and permission-set assignments in one transaction.
5. Mark the invitation accepted.
6. Create an admin session (same cookie mechanism as sign-in) and redirect to
   `/admin`.
7. Audit: `invitation.accepted` with the invitation id and user id.

### Existing accounts (phase 2)

An authenticated existing user can accept an invitation addressed to their email.
The token adds the membership and sets; it never changes their password. This
requires a signed-in session before acceptance.

## Admin UI

**`/admin/invitations`** (capability `users.manage`)

- List pending invitations: email, permission sets, created by, expires, revoke.
- Create form: email, name, sets, expiry.
- After creation, show the link once with a copy button and a warning:

  > This link will not be shown again. Anyone with it can join as the selected
  > permission sets until it expires or is revoked. Send it over a channel you
  > trust.

- Accepted/revoked invitations remain visible for audit, not linkable.
- Users page shows "Invited" state until accepted.

**`/invite/<token>`** (public, unauthenticated)

- Invalid/expired/used token: a neutral page that does not reveal whether the
  invitation ever existed. "This invitation is not valid or has expired."
- Valid: organisation name, invited email, name field, password fields, sets
  summary (read-only).
- Errors are specific for password rules, neutral for token problems.

## Security checklist

- Token hash at rest; plaintext never stored or logged.
- No token in referrer: `Referrer-Policy: no-referrer` on the acceptance page.
- No token in query strings written to access logs: configure the reverse proxy
  to drop the query string for `/invite/*`.
- Single-use atomic acceptance (`UPDATE ... WHERE accepted_at IS NULL`).
- Expiry checked on every request, not only at creation.
- Revocation is immediate.
- Invitations are tenant-scoped; a token cannot grant access to another tenant.
- Permission sets are resolved at acceptance and recorded in the audit log.
- The inviter cannot grant more capabilities than they hold (unless owner).
- Rate limiting and CAPTCHA-like protection can be added without changing the
  token model.

## API (planned)

| Method | Path | Auth | Purpose |
|---|---|---|---|
| `POST` | `/api/v1/invitations` | `users.manage` | Create, returns link once |
| `GET` | `/api/v1/invitations` | `users.manage` | List pending/accepted |
| `POST` | `/api/v1/invitations/{id}/revoke` | `users.manage` | Revoke |
| `GET` | `/api/v1/invite/{token}` | public | Validate for the form |
| `POST` | `/api/v1/invite/{token}/accept` | public | Set password, join |

## Out of scope (v1)

- Email delivery and reminders.
- Re-sending the same link (must revoke/reissue).
- Inviting an existing account without signing in.
- Domain-based auto-join.
- SCIM provisioning.
