-- Unify organisations.
--
-- The tenant, internal branches and external counterparties (providers,
-- brokers, agencies) are all rows in `organisations`. Every row is scoped to a
-- tenant root via `tenant_id`; the root points at itself.
--
-- Links between contacts and organisations, and between organisations
-- themselves, live exclusively in `relationships`.
--
-- `organisation_members` is renamed to `organisation_users`: it is
-- authentication membership (which users may sign in and with which role), not
-- part of the care data model.

-- ---------------------------------------------------------------------------
-- 1. Organisations gain the attributes previously kept on external_organisations
-- ---------------------------------------------------------------------------

ALTER TABLE organisations
    ADD COLUMN tenant_id  uuid,
    ADD COLUMN kind       text,
    ADD COLUMN abn        text,
    ADD COLUMN email      text,
    ADD COLUMN phone      text,
    ADD COLUMN website    text,
    ADD COLUMN notes      text,
    ADD COLUMN is_active  boolean NOT NULL DEFAULT true;

UPDATE organisations SET tenant_id = id;

ALTER TABLE organisations ALTER COLUMN tenant_id SET NOT NULL;

-- A new organisation is a tenant root unless it is explicitly created inside an
-- existing tenant (external provider, branch, ...).
CREATE OR REPLACE FUNCTION set_organisation_tenant() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id IS NULL THEN
        NEW.tenant_id := NEW.id;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_set_tenant
    BEFORE INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION set_organisation_tenant();

-- External names were allowed to be slightly longer than tenant names.
ALTER TABLE organisations DROP CONSTRAINT organisations_name_check;
ALTER TABLE organisations
    ADD CONSTRAINT organisations_name_check
        CHECK (char_length(name) BETWEEN 1 AND 200);

-- Only the tenant root needs a URL slug; counterparties do not.
ALTER TABLE organisations ALTER COLUMN slug DROP NOT NULL;

ALTER TABLE organisations
    ADD CONSTRAINT organisations_tenant_fk
        FOREIGN KEY (tenant_id) REFERENCES organisations (id) ON DELETE CASCADE,
    ADD CONSTRAINT organisations_tenant_id_key UNIQUE (tenant_id, id),
    ADD CONSTRAINT organisations_abn_format
        CHECK (abn IS NULL OR abn ~ '^[0-9]{11}$'),
    ADD CONSTRAINT organisations_root_requires_slug
        CHECK (tenant_id <> id OR slug IS NOT NULL);

CREATE INDEX organisations_tenant_idx ON organisations (tenant_id);
CREATE INDEX organisations_tenant_name_idx
    ON organisations (tenant_id, lower(name));

-- ---------------------------------------------------------------------------
-- 2. Relationships point at organisations instead of external_organisations
-- ---------------------------------------------------------------------------

ALTER TABLE relationships
    DROP CONSTRAINT relationships_from_organisation_fk,
    DROP CONSTRAINT relationships_to_organisation_fk;

ALTER TABLE relationships
    RENAME COLUMN from_external_organisation_id TO from_organisation_id;
ALTER TABLE relationships
    RENAME COLUMN to_external_organisation_id TO to_organisation_id;

ALTER INDEX relationships_from_org_idx RENAME TO relationships_from_organisation_idx;
ALTER INDEX relationships_to_org_idx RENAME TO relationships_to_organisation_idx;

-- ---------------------------------------------------------------------------
-- 3. Fold existing counterparties into organisations, then drop the old table
-- ---------------------------------------------------------------------------

INSERT INTO organisations (
    id, tenant_id, slug, name, edition,
    kind, abn, email, phone, website, notes, is_active,
    created_at, updated_at
)
SELECT
    id, organisation_id, NULL, name, 'community',
    kind, abn, email, phone, website, notes, is_active,
    created_at, updated_at
FROM external_organisations;

DROP TABLE external_organisations;

ALTER TABLE relationships
    ADD CONSTRAINT relationships_from_organisation_fk
        FOREIGN KEY (organisation_id, from_organisation_id)
        REFERENCES organisations (tenant_id, id) ON DELETE CASCADE,
    ADD CONSTRAINT relationships_to_organisation_fk
        FOREIGN KEY (organisation_id, to_organisation_id)
        REFERENCES organisations (tenant_id, id) ON DELETE CASCADE;

-- ---------------------------------------------------------------------------
-- 4. Authentication membership is about users, not care-model members
-- ---------------------------------------------------------------------------

ALTER TABLE organisation_members RENAME TO organisation_users;
ALTER INDEX organisation_members_user_idx RENAME TO organisation_users_user_idx;
ALTER TABLE organisation_users RENAME CONSTRAINT organisation_members_pkey TO organisation_users_pkey;
ALTER TRIGGER organisation_members_set_updated_at ON organisation_users
    RENAME TO organisation_users_set_updated_at;

-- ---------------------------------------------------------------------------
-- 5. Comments shown in the database explorer
-- ---------------------------------------------------------------------------

COMMENT ON TABLE organisations IS
    'Every organisation: the tenant itself (tenant_id = id), internal branches and external counterparties such as providers, brokers and agencies. Connected to contacts and each other through relationships.';
COMMENT ON COLUMN organisations.tenant_id IS
    'Tenant root this organisation belongs to. The root row points at itself.';
COMMENT ON COLUMN organisations.kind IS
    'Free-text descriptor, e.g. provider, broker, health, agency. Intentionally not an enum.';
COMMENT ON TABLE relationships IS
    'Typed directed links between contacts and/or organisations. Roles such as guardian, doctor, case manager or broker live here rather than on the contact record.';
COMMENT ON TABLE organisation_users IS
    'Authentication membership: which users may sign in to a tenant and with which role. Not part of the care data model.';
