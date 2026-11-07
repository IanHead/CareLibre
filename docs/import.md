# CSV import

Self-service data migration with downloadable templates and column specs.

CSV first because every system can export it, providers already work in
spreadsheets, and it keeps the server free of third-party connector credentials.
Connectors (Xero, QuickBooks, legacy CRM APIs) can reuse the same validation and
commit pipeline later.

## Implementation status (v1)

Built:

- templates and column specs for all eight objects, downloadable from
  `/admin/import`;
- upload, dry-run validation and row-level error CSV;
- staged review then all-or-nothing commit;
- `external_id` keys with per-tenant uniqueness;
- create-only and update-by-external-id policies;
- capability gate `data.import`, CSRF on mutations, audit entries;
- 10,000-row / 20 MB caps and seven-day staging expiry.

Not built yet:

- custom-field columns (`cf_<key>`) and shifts/assignments;
- partial-success imports and background processing;
- scheduled cleanup job for expired staged rows (rows expire logically but are
  not yet purged automatically);
- direct connectors.

## Principles

1. **The template is the contract.** If a column is not in the published spec,
   the import rejects it rather than silently ignoring data.
2. **Stable external IDs.** Every importable object gets an
   `external_id` so re-imports are idempotent and relationships can be resolved.
3. **Dry run always.** The operator sees counts and row-level errors before
   anything is written.
4. **All-or-nothing in v1.** One bad row fails the batch and writes nothing. This
   is slower to correct but impossible to half-apply.
5. **No PII in logs.** Filenames, hashes, counts and error positions are logged;
   values are not.
6. **Same validation as the API.** Import calls the same domain rules as normal
   writes; it is not a back door.

## Scope (v1)

| Object | Importable | Notes |
|---|---|---|
| Divisions | yes | parents may appear after children; two-pass resolution |
| Cost centres | yes | same parent rule |
| Contacts | yes | clients, workers, contacts are all the same shape |
| Organisations | yes | internal/external share one table |
| Relationships | yes | endpoints resolved by `external_id` |
| Services | yes | funding scheme + code |
| Rates | yes | belongs to a service |
| Rate versions | yes | effective dates, overlap rejected |
| Shifts / assignments | no | date-sensitive; phase 2 |
| Custom field values | no | phase 2 (`cf_<key>` columns) |
| Users / permission sets | no | invitations are the access path |

## Format rules

- UTF-8, comma-delimited, RFC 4180 quoting (`"` doubled inside quotes).
- One header row, exact column names from the spec.
- Maximum 10,000 data rows and 20 MB per file.
- Empty cell = null. Do not write `NULL` or `N/A`.
- Dates `YYYY-MM-DD`; timestamps RFC 3339 (`2026-07-01T09:00:00+10:00`).
- Booleans `true` or `false`.
- Decimals use `.`; no currency symbols or thousands separators.
- `external_id`: `A-Z`, `a-z`, `0-9`, `.`, `_`, `-`; 1–64 characters; unique per
  object per organisation; stable across re-imports.
- Unknown columns are an error (catches typos and copy/paste drift).

## Templates

Each template is downloadable from `/admin/import` as CSV with these headers plus
one clearly fake example row. The example row is never imported automatically.

### Contacts — `contacts.csv`

```csv
external_id,display_name,given_name,family_name,preferred_name,pronouns,date_of_birth,email,phone,notes,is_active
C-0001,Jane Example,Jane,Example,Janie,she/her,1980-04-12,jane.example@example.com,0400 000 000,Example row,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `display_name` | yes unless given+family | 1–160 chars |
| `given_name`, `family_name`, `preferred_name` | no | 1–160 chars |
| `pronouns` | no | free text |
| `date_of_birth` | no | valid date, not in the future |
| `email` | no | valid address, unique warning |
| `phone` | no | free text |
| `notes` | no | free text |
| `is_active` | no (default `true`) | `true` / `false` |

### Organisations — `organisations.csv`

```csv
external_id,name,kind,abn,email,phone,website,notes,is_active
O-0001,Example Broker,broker,12345678901,contact@example.com,0400 000 000,https://example.com,Example row,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `name` | yes | 1–200 chars |
| `kind` | no | free text (`provider`, `broker`, `health`, …) |
| `abn` | no | exactly 11 digits, no spaces |
| `email`, `phone`, `website` | no | email format; URL with scheme |
| `is_active` | no (default `true`) | boolean |

### Relationships — `relationships.csv`

```csv
from_type,from_external_id,to_type,to_external_id,relationship_type,inverse_name,start_date,end_date,notes
contact,C-0001,organisation,O-0001,case_manager,client,2026-07-01,,Example row
```

| Column | Required | Rules |
|---|---|---|
| `from_type` | yes | `contact` or `organisation` |
| `from_external_id` | yes | must resolve in the org (imported or existing) |
| `to_type` | yes | `contact` or `organisation` |
| `to_external_id` | yes | must resolve |
| `relationship_type` | yes | 1–64 chars |
| `inverse_name` | no | free text |
| `start_date`, `end_date` | no | dates; `end_date >= start_date` |
| `notes` | no | free text |

Relationships have no `external_id`; duplicates are detected by the full endpoint
pair + type + start date.

### Divisions — `divisions.csv`

```csv
external_id,parent_external_id,name,code,description,is_active
D-0001,,North,NOTH,Example row,true
D-0002,D-0001,North Respite,NRES,Child of D-0001,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `parent_external_id` | no | must resolve within the file or organisation; cycles rejected |
| `name` | yes | 1–160 chars |
| `code` | no | 1–32 chars; unique per organisation |
| `is_active` | no (default `true`) | boolean |

### Cost centres — `cost_centres.csv`

```csv
external_id,parent_external_id,code,name,description,external_system,external_code,is_active
CC-100,,CC-100,Programs,Example row,xero,PROG-100,true
CC-110,CC-100,CC-110,Respite,Child of CC-100,xero,PROG-110,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `parent_external_id` | no | resolves; cycles rejected |
| `code` | yes | 1–32 chars; unique per organisation |
| `name` | yes | 1–160 chars |
| `external_system` | no | `xero`, `quickbooks`, `myob`, … |
| `external_code` | no | accounting system identifier |
| `is_active` | no (default `true`) | boolean |

### Services — `services.csv`

```csv
external_id,division_external_id,code,name,description,funding_scheme,funding_code,is_active
S-001,D-0001,PC-01,Personal care,Example row,ndis,01_011_0107_1_1,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `division_external_id` | no | resolves to a division |
| `code` | yes | 1–64 chars; unique per organisation |
| `name` | yes | 1–200 chars |
| `funding_scheme` | yes | `ndis`, `aged_care`, `other` |
| `funding_code` | no | free text |
| `is_active` | no (default `true`) | boolean |

### Rates — `rates.csv`

```csv
external_id,service_external_id,name,unit_of_measure,description,is_active
R-001,S-001,Standard weekday,hour,Example row,true
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `service_external_id` | yes | resolves to a service |
| `name` | yes | 1–160 chars; unique per service |
| `unit_of_measure` | no (default `hour`) | free text (`hour`, `day`, `each`, …) |
| `is_active` | no (default `true`) | boolean |

### Rate versions — `rate_versions.csv`

```csv
external_id,rate_external_id,amount,currency,effective_from,effective_to,notes
RV-001,R-001,98.65,AUD,2026-07-01,2027-07-01,2026-27 schedule
```

| Column | Required | Rules |
|---|---|---|
| `external_id` | yes | stable key |
| `rate_external_id` | yes | resolves to a rate |
| `amount` | yes | decimal ≥ 0, two decimal places |
| `currency` | no (default `AUD`) | 3-letter code |
| `effective_from` | yes | date |
| `effective_to` | no | date, exclusive; empty = open-ended |
| `notes` | no | free text |

Overlaps are rejected against the database and against other rows in the same
file. Empty `effective_to` is allowed only once per rate.

## Wizard

```
/admin/import
  1. Choose object            → template + spec download
  2. Upload CSV               → encoding, size, header checks
  3. Map columns              → auto-match, manual override, required fields
  4. Dry run                  → would-create / would-update / rejected + error CSV
  5. Conflict policy          → create-only (default) | update by external_id
  6. Commit                   → all-or-nothing transaction, audit entry
  7. Summary                  → counts, duration, download errors, next import
```

Column mapping is saved per organisation and object to speed up repeat imports.
Mappings are portable only when headers match; the template headers are the
recommended baseline.

## Validation

Every row is validated with the same rules as the API, plus import-specific
checks:

- required columns present and non-empty;
- `external_id` unique within the file and (for create-only) absent from the org;
- foreign references resolve within the file or existing data;
- parents precede/are resolvable in a second pass; cycles rejected;
- relationship endpoints are in the same tenant and not self-referencing;
- rate-version ranges do not overlap within the file or the database;
- natural-key duplicates are reported as warnings (same email, ABN or name);
- `is_active` parses as a boolean.

Error CSV columns: `row_number,column,message,value`. Values are included so the
operator can fix them; the file is tenant-scoped and short-lived.

## Conflict policy

| Policy | Behaviour |
|---|---|
| `create_only` (default) | Any existing `external_id` or natural-key conflict fails the batch |
| `update_by_external_id` | Rows with a known `external_id` update mutable fields; unknown IDs create; natural-key collisions still fail |

Never silently overwrite, never auto-merge contacts by email. Matching is only by
`external_id`; natural keys produce warnings.

## Data model

```sql
CREATE TABLE import_batches (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    object_type      text NOT NULL,
    filename         text NOT NULL,
    file_sha256      text NOT NULL,
    row_count        integer NOT NULL DEFAULT 0,
    created_count    integer NOT NULL DEFAULT 0,
    updated_count    integer NOT NULL DEFAULT 0,
    error_count      integer NOT NULL DEFAULT 0,
    conflict_policy  text NOT NULL DEFAULT 'create_only'
                     CHECK (conflict_policy IN ('create_only', 'update_by_external_id')),
    status           text NOT NULL DEFAULT 'uploaded'
                     CHECK (status IN ('uploaded', 'validated', 'committed', 'failed', 'cancelled')),
    mapping          jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    committed_at     timestamptz
);

CREATE TABLE import_batch_errors (
    id          bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    batch_id    uuid NOT NULL REFERENCES import_batches (id) ON DELETE CASCADE,
    row_number  integer NOT NULL,
    column_name text,
    message     text NOT NULL,
    raw_value   text,
    created_at  timestamptz NOT NULL DEFAULT now()
);
```

`external_id` columns are added to `contacts`, `organisations`, `divisions`,
`cost_centres`, `services`, `rates` and `rate_versions`, with a partial unique
index per organisation.

## Security

- Requires the `data.import` capability.
- File processed in memory; no temp file left on disk.
- Logs contain object type, row counts, hash and duration — never values.
- Error CSVs are tenant-scoped, downloadable only by `data.import` holders, and
  purged after a configurable window (default 7 days).
- Import commits are audited with hash, counts and policy.
- Rate/size caps prevent accidental denial of service.

## Out of scope (v1)

- Custom field values (`cf_<key>`), shifts and assignments.
- Partial success (phase 2, explicit opt-in).
- Background/queued imports; v1 is synchronous up to 10,000 rows.
- Direct connectors and scheduled syncs.
- Undo. The remedy is a corrective import plus the audit trail.

## Acceptance criteria

- Templates download with exact headers and a clearly fake example row.
- A file with one invalid row writes nothing and returns row-level errors.
- Re-importing the same file with `create_only` fails cleanly instead of
  duplicating.
- `update_by_external_id` updates only mutable fields and never changes IDs.
- Relationships resolve endpoints imported earlier in the same file.
- Rate-version overlap is rejected with the conflicting row identified.
- No cross-tenant reference can be created.
- Every commit writes an audit entry and stores the file hash.
