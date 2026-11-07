# ADR 0007 — Custom fields are metadata, never schema

- **Status:** Accepted
- **Date:** 2026-11-07

## Context

Care providers differ in the details they track. They need to add fields
without a developer, but giving users `ALTER TABLE` (or generated columns)
would let a single tenant break migrations, indexes and every other tenant on
the same deployment. The brief also asks for per-field validation rules and
warnings about destructive changes.

## Decision

1. **Definitions are rows** in `custom_field_definitions`. Supported objects are
   listed in `custom_field_entities`; adding a new object type is an `INSERT`.
2. **Values are rows** in `custom_field_values` as JSONB, keyed by
   `(organisation_id, definition_id, entity_id)`.
3. **Types are whitelisted**: text, long text, integer, decimal, boolean, date,
   datetime, email, phone, url, single choice, multiple choice.
4. **Validation rules are structured columns** (lengths, numeric bounds, regex,
   options), not free-form SQL. Regexes are compiled by Rust at save time.
5. **Two layers of enforcement**:
   - application validation (`crate::custom_fields`) for good error messages;
   - a database trigger that rejects values whose JSON shape contradicts the
     definition, plus a composite FK that binds a definition to one object type.
6. **Shape immutability**: object type and data type cannot change once values
   exist. The database raises an error and the UI locks the controls.
7. **Archive before delete**: `is_active` hides a field but keeps values.
   Permanent deletion cascades values and requires a CSRF-protected form with an
   explicit checkbox and typed key.
8. **No user-supplied SQL** anywhere in the feature.

## Consequences

**Positive**

- A tenant can extend every object without a deploy and without risking the
  schema.
- Validation is identical in the admin UI and (future) API because both call the
  same Rust module.
- Deleting or reshaping a field cannot silently corrupt stored data.

**Negative**

- JSONB values are less convenient to index and report on than real columns.
  Hot fields should be promoted to first-class columns.
- Changing validation rules does not retro-validate existing values; a cleanup
  is an operator responsibility.
- Custom fields accumulate. The UI carries a standing warning and the docs list
  the pitfalls, but governance is social, not technical.

## Alternatives rejected

- **DDL per tenant** (`ALTER TABLE ... ADD COLUMN`): unbounded schema growth,
  migration conflicts, and unacceptable blast radius.
- **A JSONB blob per record**: simpler, but no per-field rules, no cross-object
  constraint, and no way to report on field usage.
- **EAV with typed columns**: more query-friendly than JSONB but multiplies
  columns and still needs the same definition/validation layer.
