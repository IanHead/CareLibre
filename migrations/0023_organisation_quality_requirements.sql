-- Organisation-wide worker requirements.
--
-- Some checks are not client-specific: every worker must hold a valid NDIS
-- Worker Screening check, Police check or Working With Children Check before
-- they can be rostered at all. These requirements live at the organisation
-- level and are evaluated against every candidate alongside client-specific
-- requirements.
--
-- The predicate model is the same as client_quality_requirements: the worker's
-- value must satisfy `operator`/`value`, and an unmet requirement warns and
-- ranks lower unless severity is `block`.

CREATE TABLE organisation_quality_requirements (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    quality_id       uuid NOT NULL,
    operator         text NOT NULL CHECK (operator IN (
        'exists', 'equals', 'not_equals', 'in', 'contains'
    )),
    value            jsonb,
    severity         text NOT NULL DEFAULT 'warn' CHECK (severity IN ('warn', 'block')),
    reason           text,
    legal_basis      text,
    start_date       date,
    end_date         date,
    created_by       uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT organisation_quality_requirements_id_org_key
        UNIQUE (organisation_id, id),
    CONSTRAINT organisation_quality_requirements_quality_key
        UNIQUE (organisation_id, quality_id),
    CONSTRAINT organisation_quality_requirements_quality_fk
        FOREIGN KEY (organisation_id, quality_id)
        REFERENCES worker_qualities (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT organisation_quality_requirements_value_required CHECK (
        (operator = 'exists' AND value IS NULL)
        OR (operator <> 'exists' AND value IS NOT NULL)
    ),
    CONSTRAINT organisation_quality_requirements_dates CHECK (
        end_date IS NULL OR start_date IS NULL OR end_date >= start_date
    )
);

CREATE INDEX organisation_quality_requirements_org_idx
    ON organisation_quality_requirements (organisation_id, quality_id);

CREATE TRIGGER organisation_quality_requirements_set_updated_at
    BEFORE UPDATE ON organisation_quality_requirements
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE organisation_quality_requirements IS
    'Requirements every worker must meet, regardless of client. Evaluated with client_quality_requirements during suitability.';
COMMENT ON COLUMN organisation_quality_requirements.severity IS
    'warn = advisory and ranked lower; block = hard exclusion. Screening checks are usually warn or block, by policy.';
COMMENT ON COLUMN organisation_quality_requirements.reason IS
    'Why the organisation requires this, e.g. "NDIS practice standard" or "Working with children".';

-- Add the standard screening checks to the catalogue. They are ordinary
-- quality_value qualities: a worker holds one with an expiry and evidence, and
-- an organisation requirement decides whether every worker must hold it.
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
             NULL::jsonb, 'quality_value', NULL, true, false, false),

            ('ndis_worker_screening', 'NDIS Worker Screening Check', 'screening', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('police_check', 'Police check', 'screening', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false),
            ('working_with_children_check', 'Working With Children Check', 'screening', 'boolean',
             NULL::jsonb, 'quality_value', NULL, true, false, false)
    ) AS seed (key, name, category, data_type, options, storage, contact_field,
               tracks_expiry, is_sensitive, is_system)
    ON CONFLICT DO NOTHING;
$$;

-- Existing tenants get the new screening checks; new tenants get them from the
-- organisations insert trigger already created in 0022.
SELECT carelibre_seed_worker_qualities(id) FROM organisations WHERE tenant_id = id;
