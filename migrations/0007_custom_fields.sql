-- Custom fields: user-defined metadata for every object.
--
-- Safety model:
--   * Definitions are rows, never DDL. Nothing a user does here can alter a
--     table or break another feature's schema.
--   * Values live in one table as JSONB and are checked against the definition's
--     type and options by a trigger.
--   * Data type and object type are immutable once values exist (trigger).
--   * Deletion cascades values; the admin UI requires an explicit confirmation.

CREATE TABLE custom_field_entities (
    entity_type  text PRIMARY KEY CHECK (char_length(entity_type) BETWEEN 1 AND 64),
    label        text NOT NULL,
    sort_order   integer NOT NULL DEFAULT 0
);

COMMENT ON TABLE custom_field_entities IS
    'Objects that can carry custom fields. Adding a new object type is an INSERT, not a schema change.';

INSERT INTO custom_field_entities (entity_type, label, sort_order) VALUES
    ('organisation',      'Organisations',      10),
    ('contact',           'Contacts',           20),
    ('relationship',      'Relationships',      30),
    ('division',          'Divisions',          40),
    ('cost_centre',       'Cost centres',       50),
    ('service',           'Services',           60),
    ('rate',              'Rates',              70),
    ('rate_version',      'Rate versions',      80),
    ('shift',             'Shifts',             90),
    ('shift_session',     'Shift sessions',    100),
    ('client_assignment', 'Client assignments', 110),
    ('worker_assignment', 'Worker assignments', 120);

CREATE TABLE custom_field_definitions (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    entity_type      text NOT NULL REFERENCES custom_field_entities (entity_type),
    key              text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{1,62}$'),
    label            text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 120),
    help_text        text,
    data_type        text NOT NULL CHECK (data_type IN (
                         'text', 'long_text', 'integer', 'decimal', 'boolean',
                         'date', 'datetime', 'email', 'phone', 'url',
                         'select', 'multiselect'
                     )),
    is_required      boolean NOT NULL DEFAULT false,
    min_length       integer CHECK (min_length IS NULL OR min_length >= 0),
    max_length       integer CHECK (max_length IS NULL OR max_length >= 0),
    min_value        numeric(24, 6),
    max_value        numeric(24, 6),
    pattern          text,
    options          jsonb,
    default_value    jsonb,
    sort_order       integer NOT NULL DEFAULT 0,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT custom_field_definitions_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT custom_field_definitions_key_key UNIQUE (organisation_id, entity_type, key),
    -- Required by custom_field_values so a value can prove it belongs to a
    -- definition for the same object type.
    CONSTRAINT custom_field_definitions_id_entity_key UNIQUE (organisation_id, entity_type, id),
    CONSTRAINT custom_field_definitions_length_order
        CHECK (max_length IS NULL OR min_length IS NULL OR max_length >= min_length),
    CONSTRAINT custom_field_definitions_value_order
        CHECK (max_value IS NULL OR min_value IS NULL OR max_value >= min_value),
    CONSTRAINT custom_field_definitions_length_applicable
        CHECK ((min_length IS NULL AND max_length IS NULL)
               OR data_type IN ('text', 'long_text', 'email', 'phone', 'url')),
    CONSTRAINT custom_field_definitions_range_applicable
        CHECK ((min_value IS NULL AND max_value IS NULL)
               OR data_type IN ('integer', 'decimal')),
    CONSTRAINT custom_field_definitions_pattern_applicable
        CHECK (pattern IS NULL OR data_type IN ('text', 'long_text', 'email', 'phone', 'url')),
    CONSTRAINT custom_field_definitions_options_required
        CHECK (
            (data_type IN ('select', 'multiselect')
             AND options IS NOT NULL
             AND jsonb_typeof(options) = 'array'
             AND jsonb_array_length(options) BETWEEN 1 AND 200)
            OR
            (data_type NOT IN ('select', 'multiselect') AND options IS NULL)
        )
);

CREATE INDEX custom_field_definitions_org_entity_idx
    ON custom_field_definitions (organisation_id, entity_type, sort_order, key);

COMMENT ON TABLE custom_field_definitions IS
    'User-defined field metadata. Definitions are rows; adding one never changes the database schema.';
COMMENT ON COLUMN custom_field_definitions.key IS
    'Stable machine key, e.g. ndis_number. Reserved keys such as id are rejected by the application.';
COMMENT ON COLUMN custom_field_definitions.options IS
    'JSON array of allowed strings for single/multiple choice fields.';
COMMENT ON COLUMN custom_field_definitions.is_active IS
    'Soft archive. Inactive fields keep their values but are hidden from new forms.';

CREATE TABLE custom_field_values (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    definition_id    uuid NOT NULL,
    entity_type      text NOT NULL,
    entity_id        uuid NOT NULL,
    value            jsonb NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT custom_field_values_unique
        UNIQUE (organisation_id, definition_id, entity_id),
    CONSTRAINT custom_field_values_definition_fk
        FOREIGN KEY (organisation_id, entity_type, definition_id)
        REFERENCES custom_field_definitions (organisation_id, entity_type, id)
        ON DELETE CASCADE
);

CREATE INDEX custom_field_values_entity_idx
    ON custom_field_values (organisation_id, entity_type, entity_id);

COMMENT ON TABLE custom_field_values IS
    'Values for custom fields, stored as JSONB and validated against the definition by trigger.';

-- ---------------------------------------------------------------------------
-- Type and option enforcement for values
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION validate_custom_field_value() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_type    text;
    v_options jsonb;
    v_text    text;
BEGIN
    SELECT data_type, options
    INTO v_type, v_options
    FROM custom_field_definitions
    WHERE id = NEW.definition_id
      AND organisation_id = NEW.organisation_id;

    IF v_type IS NULL THEN
        RAISE EXCEPTION 'custom field definition % not found', NEW.definition_id;
    END IF;

    -- JSON null means "no value"; requiredness is enforced by the application.
    IF NEW.value = 'null'::jsonb THEN
        RETURN NEW;
    END IF;

    IF v_type = 'integer' THEN
        IF jsonb_typeof(NEW.value) <> 'number'
           OR (NEW.value::text !~ '^-?[0-9]+$') THEN
            RAISE EXCEPTION 'custom field % expects a whole number', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
    ELSIF v_type = 'decimal' THEN
        IF jsonb_typeof(NEW.value) <> 'number' THEN
            RAISE EXCEPTION 'custom field % expects a number', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
    ELSIF v_type = 'boolean' THEN
        IF jsonb_typeof(NEW.value) <> 'boolean' THEN
            RAISE EXCEPTION 'custom field % expects a boolean', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
    ELSIF v_type = 'multiselect' THEN
        IF jsonb_typeof(NEW.value) <> 'array' THEN
            RAISE EXCEPTION 'custom field % expects an array', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
        IF EXISTS (
            SELECT 1
            FROM jsonb_array_elements_text(NEW.value) AS item
            WHERE NOT EXISTS (
                SELECT 1
                FROM jsonb_array_elements_text(v_options) AS option
                WHERE option = item
            )
        ) THEN
            RAISE EXCEPTION 'custom field % contains a value outside its options', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
    ELSE
        IF jsonb_typeof(NEW.value) <> 'string' THEN
            RAISE EXCEPTION 'custom field % expects text', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;

        v_text := NEW.value #>> '{}';

        IF v_type = 'date' AND v_text !~ '^\d{4}-\d{2}-\d{2}$' THEN
            RAISE EXCEPTION 'custom field % expects a date (YYYY-MM-DD)', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;

        IF v_type = 'datetime' AND v_text !~ '^\d{4}-\d{2}-\d{2}T' THEN
            RAISE EXCEPTION 'custom field % expects an ISO date and time', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;

        IF v_type = 'select' AND NOT EXISTS (
            SELECT 1
            FROM jsonb_array_elements_text(v_options) AS option
            WHERE option = v_text
        ) THEN
            RAISE EXCEPTION 'custom field % contains a value outside its options', NEW.definition_id
                USING ERRCODE = 'invalid_parameter_value';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER custom_field_values_validate
    BEFORE INSERT OR UPDATE ON custom_field_values
    FOR EACH ROW EXECUTE FUNCTION validate_custom_field_value();

-- Changing the shape of a field with stored values would silently corrupt
-- meaning; make it impossible rather than merely discouraged.
CREATE OR REPLACE FUNCTION guard_custom_field_definition_update() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF (NEW.entity_type <> OLD.entity_type OR NEW.data_type <> OLD.data_type)
       AND EXISTS (
           SELECT 1 FROM custom_field_values WHERE definition_id = OLD.id
       )
    THEN
        RAISE EXCEPTION
            'cannot change the object type or data type of a custom field that already has values'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER custom_field_definitions_guard
    BEFORE UPDATE ON custom_field_definitions
    FOR EACH ROW EXECUTE FUNCTION guard_custom_field_definition_update();

CREATE TRIGGER custom_field_definitions_set_updated_at
    BEFORE UPDATE ON custom_field_definitions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER custom_field_values_set_updated_at
    BEFORE UPDATE ON custom_field_values
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
