-- CSV import support.
--
-- `external_id` gives every importable entity a stable key for idempotency and
-- relationship resolution. Import batches stage parsed rows temporarily; PII is
-- purged on commit/cancel and the batch expires after seven days.

ALTER TABLE contacts        ADD COLUMN external_id text;
ALTER TABLE organisations   ADD COLUMN external_id text;
ALTER TABLE divisions       ADD COLUMN external_id text;
ALTER TABLE cost_centres    ADD COLUMN external_id text;
ALTER TABLE services        ADD COLUMN external_id text;
ALTER TABLE rates           ADD COLUMN external_id text;
ALTER TABLE rate_versions   ADD COLUMN external_id text;

CREATE UNIQUE INDEX contacts_external_id_key
    ON contacts (organisation_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX organisations_external_id_key
    ON organisations (tenant_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX divisions_external_id_key
    ON divisions (organisation_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX cost_centres_external_id_key
    ON cost_centres (organisation_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX services_external_id_key
    ON services (organisation_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX rates_external_id_key
    ON rates (organisation_id, external_id) WHERE external_id IS NOT NULL;
CREATE UNIQUE INDEX rate_versions_external_id_key
    ON rate_versions (organisation_id, external_id) WHERE external_id IS NOT NULL;

COMMENT ON COLUMN contacts.external_id IS
    'Stable identifier from an external system, used for CSV import idempotency.';

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
    -- Parsed rows, held only between review and commit/cancel.
    staged_rows      jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    committed_at     timestamptz,
    expires_at       timestamptz NOT NULL DEFAULT now() + interval '7 days'
);

CREATE INDEX import_batches_org_created_idx
    ON import_batches (organisation_id, created_at DESC);

COMMENT ON TABLE import_batches IS
    'CSV import attempts. Staged rows are purged on commit/cancel and expire after seven days.';

CREATE TABLE import_batch_errors (
    id           bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    batch_id     uuid NOT NULL REFERENCES import_batches (id) ON DELETE CASCADE,
    row_number   integer NOT NULL,
    column_name  text,
    message      text NOT NULL,
    raw_value    text,
    created_at   timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX import_batch_errors_batch_idx
    ON import_batch_errors (batch_id, row_number);
