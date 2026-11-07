# ADR 0003 — REST + OpenAPI, not GraphQL

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Every client (desktop, Android, iOS, integrations) needs a stable contract. The
brief requires API access to all functions. The two realistic choices are REST
with an OpenAPI description, or GraphQL.

## Decision

**REST under `/api/v1`, with OpenAPI 3 generated from handler annotations**
(utoipa), served at `/api-docs/openapi.json` with Swagger UI at `/docs`.

## Consequences

**Positive**

- Generated SDKs for Rust, Kotlin and Swift from one spec.
- HTTP caching, status codes and proxies work as expected.
- Authorisation is per-endpoint and easy to reason about.
- Third-party integration (Xero, Power BI) is straightforward.

**Negative**

- Highly configurable forms/fields may need chattier endpoints or purpose-built
  composite responses.
- Mobile clients occasionally need several calls to assemble one screen.

**Mitigations**

- Add purpose-built read models (e.g. `GET /shifts/{id}/detail`) where the
  generic resource shape is too chatty.
- Version the API path; additive changes stay within `v1`.

A GraphQL facade remains possible later (ADR review trigger: three or more
client-side workarounds for chatty reads).
