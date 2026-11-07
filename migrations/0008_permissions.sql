-- Permission sets: capability-based authorisation with OOTB personas.
--
-- Authorisation is by capability, never by role name. Sets are tenant-scoped and
-- additive per user. The registration owner bypasses checks (`is_owner`) and the
-- last owner cannot be removed. The Administrator set is immutable.

CREATE TABLE capabilities (
    key          text PRIMARY KEY CHECK (char_length(key) BETWEEN 1 AND 64),
    category     text NOT NULL,
    label        text NOT NULL,
    description  text NOT NULL,
    sort_order   integer NOT NULL DEFAULT 0
);

COMMENT ON TABLE capabilities IS
    'Global capability catalogue. New features add rows here; permission sets reference them.';

INSERT INTO capabilities (key, category, label, description, sort_order) VALUES
    ('contacts.read',        'People',           'View people',                'View contact records', 10),
    ('contacts.write',       'People',           'Edit people',                'Create and edit contact records', 11),
    ('relationships.read',   'People',           'View relationships',         'View links between contacts and organisations', 12),
    ('relationships.write',  'People',           'Edit relationships',         'Create and edit relationships', 13),
    ('organisations.read',   'People',           'View organisations',         'View internal and external organisations', 14),
    ('organisations.write',  'People',           'Edit organisations',         'Create and edit organisations', 15),

    ('divisions.read',       'Catalogue',        'View divisions',             'View organisational units', 20),
    ('divisions.write',      'Catalogue',        'Edit divisions',             'Create and edit divisions', 21),
    ('cost_centres.read',    'Catalogue',        'View cost centres',          'View accounting dimensions', 22),
    ('cost_centres.write',   'Catalogue',        'Edit cost centres',          'Create and edit cost centres', 23),
    ('services.read',        'Catalogue',        'View services',              'View funded services', 24),
    ('services.write',       'Catalogue',        'Edit services',              'Create and edit services', 25),
    ('rates.read',           'Catalogue',        'View rates',                 'View rates and rate versions', 26),
    ('rates.write',          'Catalogue',        'Edit rates',                 'Create and edit rates and versions', 27),
    ('custom_fields.manage', 'Catalogue',        'Manage custom fields',       'Define and delete custom fields', 28),

    ('rosters.read',         'Rostering',        'View rosters',               'View shifts, sessions and assignments', 30),
    ('rosters.write',        'Rostering',        'Edit rosters',               'Create and edit shifts, sessions and assignments', 31),
    ('rosters.approve',      'Rostering',        'Approve rosters',            'Publish and approve rosters', 32),

    ('finance.read',         'Finance',          'View finances',              'View invoices, claims and financial reports', 40),
    ('finance.write',        'Finance',          'Edit finances',              'Create and edit invoices and claims', 41),

    ('hr.read',              'Human resources',  'View HR records',            'View worker compliance and HR records', 50),
    ('hr.write',             'Human resources',  'Edit HR records',            'Maintain worker compliance and HR records', 51),

    ('users.manage',         'Administration',   'Manage users',               'Invite users and assign permission sets', 60),
    ('permission_sets.manage','Administration',  'Manage permission sets',     'Create and edit permission sets', 61),
    ('settings.manage',      'Administration',   'Manage settings',            'Organisation settings and editions', 62),
    ('audit.read',           'Administration',   'View audit log',             'Read audit events and sessions', 63),
    ('reports.read',         'Administration',   'View reports',               'View operational and financial reports', 64),
    ('data.import',          'Administration',   'Import data',                'Run CSV imports', 65),
    ('integrations.manage',  'Administration',   'Manage integrations',        'Accounting and communication integrations', 66),
    ('comms.manage',         'Administration',   'Manage communications',      'SMS/email providers and templates', 67),
    ('schema.view',          'Administration',   'View schema',                'Use the database explorer', 68);

CREATE TABLE permission_sets (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 80),
    description      text,
    persona_key      text,
    is_system        boolean NOT NULL DEFAULT false,
    is_editable      boolean NOT NULL DEFAULT true,
    sort_order       integer NOT NULL DEFAULT 100,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT permission_sets_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT permission_sets_name_key UNIQUE (organisation_id, name),
    CONSTRAINT permission_sets_persona_key_check CHECK (
        persona_key IS NULL OR persona_key IN (
            'rostering', 'intake', 'human_resources', 'senior_management',
            'house_manager', 'finance', 'administrator'
        )
    )
);

CREATE UNIQUE INDEX permission_sets_persona_key
    ON permission_sets (organisation_id, persona_key)
    WHERE persona_key IS NOT NULL;

COMMENT ON TABLE permission_sets IS
    'Tenant-scoped groups of capabilities. OOTB personas are seeded per organisation and may be edited, except Administrator.';
COMMENT ON COLUMN permission_sets.is_editable IS
    'False for the immutable Administrator set.';

CREATE TABLE permission_set_capabilities (
    organisation_id   uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    capability        text NOT NULL REFERENCES capabilities (key),
    PRIMARY KEY (permission_set_id, capability),
    CONSTRAINT permission_set_capabilities_set_fk
        FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX permission_set_capabilities_capability_idx
    ON permission_set_capabilities (capability);

CREATE TABLE organisation_user_permission_sets (
    organisation_id   uuid NOT NULL,
    user_id           uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    PRIMARY KEY (organisation_id, user_id, permission_set_id),
    CONSTRAINT organisation_user_permission_sets_user_fk
        FOREIGN KEY (organisation_id, user_id)
        REFERENCES organisation_users (organisation_id, user_id) ON DELETE CASCADE,
    CONSTRAINT organisation_user_permission_sets_set_fk
        FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE RESTRICT
);

CREATE INDEX organisation_user_permission_sets_user_idx
    ON organisation_user_permission_sets (organisation_id, user_id);

-- ---------------------------------------------------------------------------
-- Owner flag
-- ---------------------------------------------------------------------------

ALTER TABLE organisation_users
    ADD COLUMN is_owner boolean NOT NULL DEFAULT false;

UPDATE organisation_users SET is_owner = (role = 'owner');

COMMENT ON COLUMN organisation_users.is_owner IS
    'Bypasses capability checks. The last owner of an organisation cannot be removed or demoted.';

-- ---------------------------------------------------------------------------
-- Seed OOTB personas for existing and future organisations
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION carelibre_seed_permission_sets(p_organisation_id uuid)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_set uuid;
BEGIN
    IF EXISTS (
        SELECT 1 FROM permission_sets WHERE organisation_id = p_organisation_id
    ) THEN
        RETURN;
    END IF;

    -- Administrator: all capabilities, immutable. Created editable, populated,
    -- then locked so the capability guard permits the inserts.
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Administrator',
         'Full access to every capability. This set cannot be edited or deleted.',
         'administrator', true, true, 999);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities;
    UPDATE permission_sets SET is_editable = false WHERE id = v_set;

    -- Rostering
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Rostering',
         'Builds and publishes rosters; reads people and catalogue data.',
         'rostering', true, true, 10);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'relationships.read', 'organisations.read',
            'divisions.read', 'cost_centres.read', 'services.read', 'rates.read',
            'rosters.read', 'rosters.write', 'reports.read'
        );

    -- Intake
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Intake',
         'Creates and links people and organisations; imports referral data.',
         'intake', true, true, 20);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'contacts.write', 'relationships.read', 'relationships.write',
            'organisations.read', 'organisations.write', 'divisions.read',
            'services.read', 'reports.read', 'data.import'
        );

    -- Human Resources
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Human Resources',
         'Maintains worker records and compliance; reads rosters.',
         'human_resources', true, true, 30);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'contacts.write', 'relationships.read',
            'organisations.read', 'divisions.read', 'cost_centres.read',
            'services.read', 'rates.read', 'rosters.read', 'hr.read', 'hr.write',
            'reports.read'
        );

    -- Senior Management
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Senior Management',
         'Read access across the organisation, roster approval and audit visibility.',
         'senior_management', true, true, 40);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'relationships.read', 'organisations.read',
            'divisions.read', 'cost_centres.read', 'services.read', 'rates.read',
            'rosters.read', 'rosters.approve', 'finance.read', 'hr.read',
            'audit.read', 'reports.read'
        );

    -- House Manager
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'House Manager',
         'Read-only access to rosters and records for a house manager.',
         'house_manager', true, true, 50);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'relationships.read', 'organisations.read',
            'divisions.read', 'cost_centres.read', 'services.read', 'rates.read',
            'rosters.read', 'reports.read'
        );

    -- Finance
    v_set := gen_random_uuid();
    INSERT INTO permission_sets
        (id, organisation_id, name, description, persona_key, is_system, is_editable, sort_order)
    VALUES
        (v_set, p_organisation_id, 'Finance',
         'Owns rates, invoices, claims and accounting integrations.',
         'finance', true, true, 60);
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        SELECT p_organisation_id, v_set, key FROM capabilities WHERE key IN (
            'contacts.read', 'relationships.read', 'organisations.read',
            'divisions.read', 'cost_centres.read', 'services.read',
            'rates.read', 'rates.write', 'rosters.read', 'finance.read',
            'finance.write', 'reports.read', 'integrations.manage'
        );
END;
$$;

SELECT carelibre_seed_permission_sets(id) FROM organisations;

-- Map legacy roles onto personas so existing members keep working.
INSERT INTO organisation_user_permission_sets (organisation_id, user_id, permission_set_id)
SELECT ou.organisation_id, ou.user_id, ps.id
FROM organisation_users ou
JOIN permission_sets ps
  ON ps.organisation_id = ou.organisation_id
 AND ps.persona_key = 'administrator'
WHERE ou.role = 'admin'
ON CONFLICT DO NOTHING;

INSERT INTO organisation_user_permission_sets (organisation_id, user_id, permission_set_id)
SELECT ou.organisation_id, ou.user_id, ps.id
FROM organisation_users ou
JOIN permission_sets ps
  ON ps.organisation_id = ou.organisation_id
 AND ps.persona_key IN ('rostering', 'intake')
WHERE ou.role = 'manager'
ON CONFLICT DO NOTHING;

INSERT INTO organisation_user_permission_sets (organisation_id, user_id, permission_set_id)
SELECT ou.organisation_id, ou.user_id, ps.id
FROM organisation_users ou
JOIN permission_sets ps
  ON ps.organisation_id = ou.organisation_id
 AND ps.persona_key = 'house_manager'
WHERE ou.role IN ('staff', 'viewer')
ON CONFLICT DO NOTHING;

-- ---------------------------------------------------------------------------
-- Guardrails
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION guard_last_owner() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_organisation uuid := COALESCE(NEW.organisation_id, OLD.organisation_id);
BEGIN
    IF (TG_OP = 'DELETE' AND OLD.is_owner)
       OR (TG_OP = 'UPDATE' AND OLD.is_owner AND NOT NEW.is_owner)
    THEN
        IF (
            SELECT count(*)
            FROM organisation_users
            WHERE organisation_id = v_organisation AND is_owner
        ) <= 1 THEN
            RAISE EXCEPTION 'cannot remove or demote the last owner of organisation %', v_organisation
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
    END IF;

    RETURN COALESCE(NEW, OLD);
END;
$$;

CREATE TRIGGER organisation_users_guard_last_owner
    BEFORE UPDATE OR DELETE ON organisation_users
    FOR EACH ROW EXECUTE FUNCTION guard_last_owner();

CREATE OR REPLACE FUNCTION guard_permission_set_write() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        IF OLD.is_system AND NOT OLD.is_editable THEN
            RAISE EXCEPTION 'the administrator permission set cannot be deleted'
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
        RETURN OLD;
    END IF;

    IF OLD.is_system AND NOT OLD.is_editable THEN
        IF NEW.name <> OLD.name
           OR NEW.description IS DISTINCT FROM OLD.description
           OR NEW.is_editable <> OLD.is_editable
        THEN
            RAISE EXCEPTION 'the administrator permission set cannot be edited'
                USING ERRCODE = 'object_not_in_prerequisite_state';
        END IF;
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER permission_sets_guard_write
    BEFORE UPDATE OR DELETE ON permission_sets
    FOR EACH ROW EXECUTE FUNCTION guard_permission_set_write();

CREATE OR REPLACE FUNCTION guard_permission_set_capabilities() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_set uuid := COALESCE(NEW.permission_set_id, OLD.permission_set_id);
BEGIN
    IF EXISTS (
        SELECT 1 FROM permission_sets
        WHERE id = v_set AND is_system AND NOT is_editable
    ) THEN
        RAISE EXCEPTION 'the administrator permission set is immutable'
            USING ERRCODE = 'object_not_in_prerequisite_state';
    END IF;

    RETURN COALESCE(NEW, OLD);
END;
$$;

CREATE TRIGGER permission_set_capabilities_guard
    BEFORE INSERT OR UPDATE OR DELETE ON permission_set_capabilities
    FOR EACH ROW EXECUTE FUNCTION guard_permission_set_capabilities();

CREATE OR REPLACE FUNCTION seed_permission_sets_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM carelibre_seed_permission_sets(NEW.id);
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_seed_permission_sets
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION seed_permission_sets_for_organisation();

CREATE TRIGGER permission_sets_set_updated_at
    BEFORE UPDATE ON permission_sets
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE organisation_user_permission_sets IS
    'Additive permission-set assignments. Effective capabilities are the union of all assigned sets, unless the user is_owner.';
