# ADR 0012 — Organisation branding stored in PostgreSQL

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Desktop and mobile clients need the organisation's display name and logo after
sign-in. The server is a single self-hosted binary backed by PostgreSQL; there is
no object-storage dependency and no guarantee that an operator mounts or backs up
an application filesystem volume.

The logo is small (a few hundred KB at most) and changes rarely. It must survive
backups, work in a single-instance deployment, and be cacheable by clients.

## Decision

1. **The name stays on `organisations`.** It is already the authoritative display
   name for the tenant root and is edited through the admin settings page and
   `PATCH /api/v1/organisation`.
2. **The logo is stored in `organisation_branding`** as `bytea`, scoped one-to-one
   to a tenant root. Counterpart organisations cannot carry branding; a trigger
   enforces this.
3. **Only small, web-safe raster formats are accepted:** PNG, JPEG, WebP and GIF,
   detected from magic bytes, with a 2 MB limit. SVG is rejected because it can
   carry active content.
4. **The logo is served by the API** at `GET /api/v1/organisation/logo` with its
   detected content type, `X-Content-Type-Options: nosniff`, and a SHA-256 ETag.
   Clients can revalidate with `If-None-Match` and receive `304`.
5. **Names and logos are edited in the admin UI** under `/admin/organisation`
   (`settings.manage`) and through the API for clients that need to automate it.
6. **Both changes are audited** in `audit_log`.

## Consequences

**Positive**

- One backup (the database) contains identity and branding; no orphaned files.
- Multi-instance deployments see the same logo immediately.
- ETag caching keeps the common client path cheap.
- The admin UI and API share one storage and validation path.

**Negative**

- Large images are not appropriate; the 2 MB cap and raster-only rule are
  deliberate.
- Binary data in PostgreSQL is less efficient than object storage at very large
  scale, but the logo is a single small object per tenant.
- Serving bytes through the API consumes application bandwidth.

## Alternatives rejected

- **Filesystem volume:** requires an operator-managed volume and breaks when
  multiple server instances do not share it.
- **External object storage (S3/R2):** adds a deployment dependency and
  credentials that the community edition should not require.
- **Base64 in a text column on `organisations`:** bloats every organisation query
  and makes accidental leakage into logs more likely.
- **SVG uploads:** flexible, but scripting and external references make them a
  poor default for untrusted admin uploads.
