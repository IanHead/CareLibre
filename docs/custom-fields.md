# Custom fields

Custom fields let an organisation extend any object with data specific to its
own practice — without a developer and without a schema migration.

They are **metadata, not DDL**. Adding a field inserts rows into
`custom_field_definitions`; it never alters a table. A user cannot damage the
database, but they can still make a mess of the *data model*. This page explains
both the mechanics and the responsibilities.

## What you can configure

| Setting | Applies to | Notes |
|---|---|---|
| Object | all | Contacts, organisations, divisions, services, rates, shifts, assignments, cost centres, relationships |
| Key | all | Stable identifier for reports/API: `a-z`, digits and `_`, starting with a letter |
| Label / help text | all | What staff see on forms |
| Data type | all | See below |
| Required | all | Enforced when values are written |
| Min/max length | text-like | Character count |
| Regular expression | text-like | Rust regex syntax |
| Min/max value | numbers | Numeric bounds |
| Options | single/multiple choice | At least one, at most 200 |
| Default value | all | Validated against the type and rules |
| Display order | all | Sort within the object |

Data types: `text`, `long_text`, `integer`, `decimal`, `boolean`, `date`,
`datetime`, `email`, `phone`, `url`, `select`, `multiselect`.

## Safety model

- Definitions are rows with validated keys and whitelisted types. There is no
  user-supplied SQL and no `ALTER TABLE`.
- Values live in `custom_field_values` as JSONB, one row per record/field.
- A database trigger checks the JSON shape against the definition's type and
  options, so a buggy client cannot store `"abc"` in a number field.
- A composite foreign key prevents attaching a contact field to a shift.
- Object type and data type are **immutable once values exist**. The database
  refuses the change; the UI locks the fields.
- Deleting a definition cascades its values. The UI shows the number of values
  and requires the operator to tick a box and type the field key.
- Archiving (`is_active = false`) is the reversible option: values are kept and
  the field is hidden from new forms.

## Using the screens

**Administration → Custom fields** (`/admin/custom-fields`)

1. *Add a custom field* — choose object, key, label, type, and rules.
2. *Edit* — change labels, rules, options and defaults. Object/type are locked
   once values exist.
3. *Archive* — reversible; hides the field but keeps data.
4. *Danger zone* — permanent delete. It removes every stored value.

All mutating forms carry a CSRF token (double-submit cookie), so cross-site
posts are rejected.

## The responsibility warning

The UI shows this on every screen, and it is worth repeating:

> Every custom field becomes part of your organisation's data contract: reports,
> exports, mobile forms and staff training must account for it. Prefer a small
> set of well-named fields over many speculative ones. If a concept applies to
> every provider, it belongs in the core model.

Practical pitfalls of adding fields freely:

- **Reporting drift.** Each field multiplies combinations in exports and
  dashboards; two similar fields (`ndis_no`, `ndis_number`) make both useless.
- **Form fatigue.** A 40-field intake form is abandoned, and the data quality
  loss outweighs the extra detail.
- **Rule churn.** Tightening validation later leaves old values that no longer
  pass. The system does not retro-validate; plan a cleanup.
- **Integrations.** Keys are part of an API contract. Renaming one breaks
  consumers even though stored values are keyed by ID.
- **False permanence.** Custom fields are not a substitute for modelling a
  first-class concept. When several providers need the same field, promote it
  into the core schema with a migration.

## API shape (planned)

Definitions and values will be exposed under `/api/v1/custom-fields` and
`/api/v1/{object}/{id}/custom-fields`. Validation uses the same Rust module
(`crate::custom_fields`) the admin UI uses, so both paths apply identical rules.
