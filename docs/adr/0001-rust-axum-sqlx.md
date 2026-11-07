# ADR 0001 — Rust, axum and sqlx for the server

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

CareLibre needs a server that is fast, self-hostable by small care providers
(one `docker compose up`), and maintainable for a decade. The desktop client is
committed to Rust (egui/wgpu) and the mobile clients will be native.

Candidate stacks: Rust (axum/actix), TypeScript (NestJS), Python (Django),
Go (chi/echo), C# (ASP.NET Core), PHP (Laravel).

## Decision

**Rust**, with:

- **axum 0.8** for HTTP. Tokio-native, small surface area, extractor model keeps
  handlers declarative, and it composes with `tower` middleware.
- **sqlx** with PostgreSQL for data access. Async, compile-time-checkable SQL
  (we start with runtime-checked queries so the crate builds without a live
  database; checked queries can be adopted per-module later), no ORM.

## Consequences

**Positive**

- One language across server and desktop; domain types can be shared later.
- Predictable latency and memory use on modest hardware — important for the
  free Community Edition running on a provider's own box.
- Strong type system and exhaustive matching on domain rules (rates, rosters).
- Single static-ish binary; no runtime dependency beyond libc.

**Negative**

- Slower iteration than TypeScript or Python; more code to ship a CRUD endpoint.
- Smaller hiring pool than PHP/.NET in the Australian care sector.
- Must provide our own admin tooling instead of getting Django admin for free.

**Mitigations**

- OpenAPI generation and a thin handler layer reduce boilerplate.
- Foundation ships conventions (error envelope, extractors) so later endpoints
  are mechanical.
