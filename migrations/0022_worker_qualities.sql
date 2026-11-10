-- Worker qualities and client requirements.
--
-- A client may require the workers who support them to hold a particular
-- skill or training (PEG feeding, first aid) or to have a particular attribute
-- (a female worker, a Cantonese speaker). These are not care relationships and
-- they are not free-form metadata: rostering has to evaluate them.
--
-- The model has three parts:
--
--   * `worker_qualities` is the catalogue. A quality is either stored as a
--     column on `contacts` (`storage = 'contact_field'`, always-known profile
--     data such as gender) or as a row in `worker_quality_values`
--     (`storage = 'quality_value'`, on-demand training such as PEG feeding).
--   * `worker_quality_values` holds what a worker actually has. A row exists
--     only when the worker holds the quality; absence means "not recorded".
--   * `client_quality_requirements` is a predicate over a quality. Severity
--     defaults to `warn`; `block` is an explicit opt-in.
--
-- Suitability reads these tables on demand and turns unmet requirements into
-- warnings and a modest penalty. Nothing is auto-assigned and nothing is
-- hard-excluded unless the requirement says `block`.

-- ---------------------------------------------------------------------------
-- 1. Intrinsic profile fields on contacts
-- ---------------------------------------------------------------------------

ALTER TABLE contacts
    ADD COLUMN gender text CHECK (
        gender IS NULL OR gender IN ('female', 'male', 'non_binary', 'prefer_not_to_say')
    ),
    ADD COLUMN languages text[] NOT NULL DEFAULT '{}',
    ADD COLUMN cultural_identity text;

CREATE INDEX contacts_org_gender_idx ON contacts (organisation_id, gender);
CREATE INDEX contacts_languages_idx ON contacts USING gin (languages);

COMMENT ON COLUMN contacts.gender IS
    'Recorded gender, entered once as ordinary profile data. Client requirements can match on it; it is never duplicated into worker_quality_values.';
COMMENT ON COLUMN contacts.languages IS
    'Languages the person speaks. Entered once as profile data; requirements can use contains.';
COMMENT ON COLUMN contacts.cultural_identity IS
    'Self-described cultural identity. Sensitive; requirements need a reason.';

-- Best-effort backfill from the intake JSONB, which was display-only before.
UPDATE contacts
SET gender = CASE lower(btrim(details ->> 'gender'))
        WHEN 'female' THEN 'female'
        WHEN 'male' THEN 'male'
        WHEN 'non-binary' THEN 'non_binary'
        WHEN 'nonbinary' THEN 'non_binary'
        WHEN 'non binary' THEN 'non_binary'
        WHEN 'prefer not to say' THEN 'prefer_not_to_say'
        WHEN 'prefer_not_to_say' THEN 'prefer_not_to_say'
        ELSE NULL
    END
WHERE gender IS NULL
  AND details ? 'gender'
  AND jsonb_typeof(details -> 'gender') = 'string';

UPDATE contacts
SET languages = (
    SELECT array_agg(btrim(lang) ORDER BY btrim(lang))
    FROM unnest(regexp_split_to_array(details ->> 'languages', ',')) AS lang
    WHERE btrim(lang) <> ''
)
WHERE cardinality(languages) = 0
  AND details ? 'languages'
  AND jsonb_typeof(details -> 'languages') = 'string';

UPDATE contacts
SET languages = ARRAY(
    SELECT btrim(value)
    FROM jsonb_array_elements_text(details -> 'languages') AS value
    WHERE btrim(value) <> ''
    ORDER BY btrim(value)
)
WHERE cardinality(languages) = 0
  AND details ? 'languages'
  AND jsonb_typeof(details -> 'languages') = 'array';

UPDATE contacts
SET cultural_identity = btrim(details ->> 'cultural_identity')
WHERE cultural_identity IS NULL
  AND details ? 'cultural_identity'
  AND jsonb_typeof(details -> 'cultural_identity') = 'string'
  AND btrim(details ->> 'cultural_identity') <> '';

-- ---------------------------------------------------------------------------
-- 2. Quality catalogue
-- ---------------------------------------------------------------------------

CREATE TABLE worker_qualities (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    key              text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{1,62}$'),
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    description      text,
    category         text NOT NULL DEFAULT 'attribute' CHECK (category IN (
        'skill', 'training', 'qualification', 'screening', 'attribute', 'preference'
    )),
    data_type        text NOT NULL DEFAULT 'boolean' CHECK (data_type IN (
        'boolean', 'select', 'multiselect', 'text', 'date'
    )),
    options          jsonb,
    storage          text NOT NULL DEFAULT 'quality_value'
                     CHECK (storage IN ('contact_field', 'quality_value')),
    contact_field    text CHECK (contact_field IN (
        'gender', 'languages', 'cultural_identity', 'date_of_birth'
    )),
    tracks_expiry    boolean NOT NULL DEFAULT false,
    is_sensitive     boolean NOT NULL DEFAULT false,
    is_system        boolean NOT NULL DEFAULT false,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT worker_qualities_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT worker_qualities_key_key UNIQUE (organisation_id, key),
    CONSTRAINT worker_qualities_storage_shape CHECK (
        (storage = 'contact_field' AND contact_field IS NOT NULL AND is_system)
        OR (storage = 'quality_value' AND contact_field IS NULL)
    ),
    CONSTRAINT worker_qualities_options_shape CHECK (
        options IS NULL OR jsonb_typeof(options) = 'array'
    )
);

CREATE INDEX worker_qualities_org_active_idx
    ON worker_qualities (organisation_id, is_active, name);

CREATE TRIGGER worker_qualities_set_updated_at
    BEFORE UPDATE ON worker_qualities
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE worker_qualities IS
    'Catalogue of things a worker can hold or be, and a client can require. Intrinsic qualities map to a contacts column; on-demand qualities live in worker_quality_values.';
COMMENT ON COLUMN worker_qualities.storage IS
    'contact_field = read the named column on contacts (system rows only); quality_value = read worker_quality_values.';
COMMENT ON COLUMN worker_qualities.is_sensitive IS
    'Sensitive attributes (gender, cultural identity). A client requirement using one must record a reason or legal basis.';

-- ---------------------------------------------------------------------------
-- 3. What a worker actually holds
-- ---------------------------------------------------------------------------

CREATE TABLE worker_quality_values (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    worker_contact_id uuid NOT NULL,
    quality_id        uuid NOT NULL,
    value             jsonb NOT NULL,
    expires_on        date,
    evidence          text,
    verified_at       timestamptz,
    verified_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    notes             text,
    removed_at        timestamptz,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT worker_quality_values_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT worker_quality_values_contact_fk
        FOREIGN KEY (organisation_id, worker_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT worker_quality_values_quality_fk
        FOREIGN KEY (organisation_id, quality_id)
        REFERENCES worker_qualities (organisation_id, id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX worker_quality_values_current_key
    ON worker_quality_values (organisation_id, worker_contact_id, quality_id)
    WHERE removed_at IS NULL;

CREATE INDEX worker_quality_values_quality_idx
    ON worker_quality_values (organisation_id, quality_id)
    WHERE removed_at IS NULL;

CREATE TRIGGER worker_quality_values_set_updated_at
    BEFORE UPDATE ON worker_quality_values
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE worker_quality_values IS
    'A quality a worker holds. A row exists only when it is relevant; absence means not recorded, never an explicit negative.';

-- ---------------------------------------------------------------------------
-- 4. What a client requires
-- ---------------------------------------------------------------------------

CREATE TABLE client_quality_requirements (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    client_contact_id uuid NOT NULL,
    quality_id        uuid NOT NULL,
    operator          text NOT NULL CHECK (operator IN (
        'exists', 'equals', 'not_equals', 'in', 'contains'
    )),
    value             jsonb,
    severity          text NOT NULL DEFAULT 'warn' CHECK (severity IN ('warn', 'block')),
    reason            text,
    legal_basis       text,
    service_id        uuid,
    start_date        date,
    end_date          date,
    created_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT client_quality_requirements_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT client_quality_requirements_contact_fk
        FOREIGN KEY (organisation_id, client_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_quality_requirements_quality_fk
        FOREIGN KEY (organisation_id, quality_id)
        REFERENCES worker_qualities (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_quality_requirements_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_quality_requirements_value_required CHECK (
        (operator = 'exists' AND value IS NULL)
        OR (operator <> 'exists' AND value IS NOT NULL)
    ),
    CONSTRAINT client_quality_requirements_dates CHECK (
        end_date IS NULL OR start_date IS NULL OR end_date >= start_date
    )
);

CREATE UNIQUE INDEX client_quality_requirements_key
    ON client_quality_requirements (
        organisation_id, client_contact_id, quality_id,
        COALESCE(service_id, '00000000-0000-0000-0000-000000000000'::uuid)
    );

CREATE INDEX client_quality_requirements_client_idx
    ON client_quality_requirements (organisation_id, client_contact_id);

CREATE TRIGGER client_quality_requirements_set_updated_at
    BEFORE UPDATE ON client_quality_requirements
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE client_quality_requirements IS
    'A predicate a client places on workers. severity = warn is the default: unmet requirements warn and rank lower but remain assignable. block is an explicit hard exclusion.';
COMMENT ON COLUMN client_quality_requirements.legal_basis IS
    'Why a sensitive requirement (for example a female worker for personal care) is justified. Required when the quality is sensitive.';
COMMENT ON COLUMN client_quality_requirements.service_id IS
    'Optional scope: the requirement applies only to shifts for this service. NULL applies to all of the client''s shifts.';

-- ---------------------------------------------------------------------------
-- 5. Expiry status view
-- ---------------------------------------------------------------------------

CREATE VIEW worker_quality_status AS
SELECT
    v.organisation_id,
    v.worker_contact_id,
    v.quality_id,
    q.name AS quality_name,
    q.data_type,
    v.value,
    v.expires_on,
    CASE
        WHEN q.tracks_expiry AND v.expires_on IS NOT NULL AND v.expires_on < CURRENT_DATE
            THEN 'expired'
        WHEN q.tracks_expiry AND v.expires_on IS NOT NULL AND v.expires_on <= CURRENT_DATE + 30
            THEN 'expiring'
        ELSE 'current'
    END AS status
FROM worker_quality_values v
JOIN worker_qualities q
  ON q.organisation_id = v.organisation_id AND q.id = v.quality_id
WHERE v.removed_at IS NULL;

COMMENT ON VIEW worker_quality_status IS
    'Current worker quality values with expiry status (current, expiring within 30 days, or expired).';

-- ---------------------------------------------------------------------------
-- 6. Seed the catalogue
-- ---------------------------------------------------------------------------

-- Intrinsic rows map to a contacts column. They are read-only: users edit the
-- profile field, never a duplicate quality value.
CREATE OR REPLACE FUNCTION carelibre_seed_worker_qualities(p_organisation_id uuid)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO worker_qualities
        (id, organisation_id, key, name, category, data_type, options,
         storage, contact_field, tracks_expiry, is_sensitive, is_system)
    SELECT gen_random_uuid(), p_organisation_id, seed.key, seed.name, seed.category,
           seed.data_type, seed.options, seed.storage, seed.contact_field,
           seed.tracks_expiry, seed.is_sensitive, seed.is_system
    FROM (
        VALUES
            ('gender', 'Gender', 'attribute', 'select',
             '["female","male","non_binary","prefer_not_to_say"]'::jsonb,
             'contact_field', 'gender', false, true, true),
            ('languages', 'Languages spoken', 'attribute', 'multiselect',
             NULL::jsonb, 'contact_field', 'languages', false, false, true),
            ('cultural_identity', 'Cultural identity', 'attribute', 'text',
             NULL::jsonb, 'contact_field', 'cultural_identity', false, true, true),

            ('peg_feeding', 'PEG feeding and enteral nutrition', 'training', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('manual_handling', 'Manual handling', 'training', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('medication_administration', 'Medication administration', 'training', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('first_aid', 'First aid', 'qualification', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('cpr', 'CPR', 'training', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('drivers_licence', 'Driver''s licence', 'qualification', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false)
    ) AS seed (key, name, category, data_type, options, storage, contact_field,
               tracks_expiry, is_sensitive, is_system)
    ON CONFLICT DO NOTHING;
$$;

-- New tenant organisations start with the standard catalogue.
CREATE OR REPLACE FUNCTION seed_worker_qualities_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_worker_qualities(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_seed_worker_qualities
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION seed_worker_qualities_for_organisation();

-- Existing tenants: the standard catalogue.
SELECT carelibre_seed_worker_qualities(id) FROM organisations WHERE tenant_id = id;

-- ---------------------------------------------------------------------------
-- 7. Capabilities
-- ---------------------------------------------------------------------------

INSERT INTO capabilities (key, category, label, description, sort_order) VALUES
    ('worker_qualities.manage', 'People', 'Manage worker qualities',
     'Add and retire skills, training and attributes in the worker quality catalogue.', 19);

ALTER TABLE permission_set_capabilities DISABLE TRIGGER permission_set_capabilities_guard;
INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, 'worker_qualities.manage'
FROM permission_sets ps
WHERE ps.persona_key = 'administrator';
ALTER TABLE permission_set_capabilities ENABLE TRIGGER permission_set_capabilities_guard;
