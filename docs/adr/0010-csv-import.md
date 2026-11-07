# ADR 0010 — CSV import with published templates and specs

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Migrating providers arrive with exports from spreadsheets or another CRM. Direct
connectors are valuable later but each one is a project with credentials, API
changes and rate limits. Providers need a dependable path on day one, and support
needs a contract they can point at when a file fails.

## Decision

1. **CSV first.** One row per record, UTF-8, RFC 4180, header row, capped at
   10,000 rows / 20 MB in v1.
2. **Published templates and column specs** for every importable object. Unknown
   columns are an error; silent column drops hide mistakes.
3. **`external_id` is mandatory** for importable entities, with a partial unique
   index per organisation. It is the idempotency and relationship-resolution key.
4. **Dry run is mandatory**, producing counts and a downloadable row-level error
   file before any write.
5. **All-or-nothing in v1.** Any invalid row fails the batch and writes nothing.
   Partial success is a later, explicit opt-in.
6. **Conflict policy is chosen explicitly**: `create_only` (default) or
   `update_by_external_id`. Natural-key matches are warnings, never auto-merges.
7. **Same validation as the API.** Import must not be a bypass for domain rules.
8. **No PII in logs.** Store filename, hash, counts and errors; never log values.
9. **Connectors later** reuse the same validation/commit pipeline.

## Consequences

**Positive**

- Any provider with a spreadsheet can migrate without waiting for a connector.
- The template doubles as documentation and support material.
- `external_id` makes re-imports safe and relationship imports possible.
- All-or-nothing prevents half-linked data that is hard to unwind.

**Negative**

- Large files require a clean-up loop; one bad row blocks 10,000 good ones until
  fixed.
- Templates must be maintained as the schema evolves; drift is a support cost.
- Natural-key conflicts require human judgement instead of automatic merging.
- Synchronous imports bound the file size in v1.

## Alternatives rejected

- **Connectors first:** slower to value, more credentials, still needs CSV for
  every system without an API.
- **Best-effort partial import:** tempting for large files, but creates orphaned
  relationships and rate gaps that are worse than a failed batch.
- **Free-form column mapping into arbitrary fields:** hides typos and encourages
  divergent files; a published spec is a better contract.
