# ADR 0009 — One-time invitation links

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

The server must be usable by a self-hosted provider with no SMTP configured. The
first administrator needs to add colleagues and assign permission sets. Email
delivery would gate onboarding on infrastructure the Community Edition may not
have. Password reset links are a related but distinct problem.

## Decision

1. **Invitations are one-time links with a 32-byte random token**, stored only as
   a SHA-256 hash.
2. **The link is shown once at creation**; losing it means revoking and
   reissuing, never retrieving.
3. **Expiry defaults to 7 days** (allowed 1–30) and the token is single-use.
4. **Acceptance is unauthenticated for new accounts**: the invitee sets a password
   and is signed in.
5. **Existing accounts cannot be taken over by an invitation.** If the email
   already has a user account, acceptance is refused and the person must sign in
   first; support for authenticated acceptance is a later phase.
6. **Permission sets are fixed on the invitation** and applied at acceptance.
7. **Security headers and log hygiene**: `Referrer-Policy: no-referrer`,
   `Cache-Control: no-store`, and no token in application logs.
8. **Tenant-scoped and audited**: composite keys, creation/revocation/acceptance
   audit entries.

Email delivery can be added later as a transport for the same token; it is not
part of the acceptance model.

## Consequences

**Positive**

- Works immediately on a fresh self-hosted install.
- The token has the same threat model as a password-reset link and is revocable.
- Permission sets travel with the invitation, so access is correct from day one.
- No dependency on an email provider for onboarding.

**Negative**

- The administrator must deliver the link themselves and handle it as a secret.
- Copy/paste and chat history are new leak channels; the "shown once" warning and
  expiry limit exposure.
- Existing-account invitations require a second flow later.

## Alternatives rejected

- **Email-only invitations:** blocks Community Edition setups without SMTP and
  makes the admin dependent on deliverability.
- **Reusable links until expiry:** converts a single invite into a standing join
  code; unacceptable for privilege-bearing links.
- **Storing the plaintext token:** unnecessary risk; hashing supports lookup by
  hash without retaining the secret.
