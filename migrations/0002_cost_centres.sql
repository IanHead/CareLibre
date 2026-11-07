-- Cost centres: hierarchical accounting dimensions.
--
-- A cost centre maps to an accounting system (Xero, QuickBooks, ...) and can be
-- applied to any record. Inheritance is configurable: `record_parents` records
-- the generic child -> parent links, and `cost_centre_inheritance_rules` defines
-- the lookup order for each entity type. `carelibre_resolve_cost_centre()`
-- walks that chain until it finds a direct assignment.

CREATE TABLE cost_centres (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    parent_id        uuid,
    code             text NOT NULL CHECK (char_length(code) BETWEEN 1 AND 32),
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    description      text,
    external_system  text,
    external_code    text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT cost_centres_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT cost_centres_code_key UNIQUE (organisation_id, code),
    CONSTRAINT cost_centres_parent_fk
        FOREIGN KEY (organisation_id, parent_id)
        REFERENCES cost_centres (organisation_id, id),
    CONSTRAINT cost_centres_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE INDEX cost_centres_org_idx ON cost_centres (organisation_id);
CREATE INDEX cost_centres_parent_idx ON cost_centres (organisation_id, parent_id);

COMMENT ON TABLE cost_centres IS
    'Hierarchical accounting dimensions. Maps to a cost centre in an accounting system and can be inherited by any descendant record.';
COMMENT ON COLUMN cost_centres.external_system IS
    'Accounting system this cost centre maps to, e.g. xero, quickbooks, myob.';
COMMENT ON COLUMN cost_centres.external_code IS
    'Identifier of this cost centre in the external accounting system.';

-- Direct assignments. Polymorphic by design: entity_id has no FK because any
-- record type may carry a cost centre. Entity types are documented in
-- docs/data-model.md.
CREATE TABLE cost_centre_assignments (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    cost_centre_id   uuid NOT NULL,
    entity_type      text NOT NULL CHECK (char_length(entity_type) BETWEEN 1 AND 64),
    entity_id        uuid NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT cost_centre_assignments_unique
        UNIQUE (organisation_id, entity_type, entity_id),
    CONSTRAINT cost_centre_assignments_cost_centre_fk
        FOREIGN KEY (organisation_id, cost_centre_id)
        REFERENCES cost_centres (organisation_id, id) ON DELETE RESTRICT
);

CREATE INDEX cost_centre_assignments_entity_idx
    ON cost_centre_assignments (organisation_id, entity_type, entity_id);
CREATE INDEX cost_centre_assignments_cost_centre_idx
    ON cost_centre_assignments (organisation_id, cost_centre_id);

COMMENT ON TABLE cost_centre_assignments IS
    'Direct cost-centre assignment for any record, keyed by (entity_type, entity_id). One cost centre per record.';

-- Generic parent links, populated when the application links records. Keeping
-- them separate from the domain FKs makes the inheritance path configurable.
CREATE TABLE record_parents (
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    child_type       text NOT NULL CHECK (char_length(child_type) BETWEEN 1 AND 64),
    child_id         uuid NOT NULL,
    parent_type      text NOT NULL CHECK (char_length(parent_type) BETWEEN 1 AND 64),
    parent_id        uuid NOT NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, child_type, child_id, parent_type),
    CONSTRAINT record_parents_not_self
        CHECK (child_type <> parent_type OR child_id <> parent_id)
);

CREATE INDEX record_parents_parent_idx
    ON record_parents (organisation_id, parent_type, parent_id);

COMMENT ON TABLE record_parents IS
    'Generic child -> parent links used by cost-centre inheritance. A child may have at most one parent of each type.';

-- Ordered inheritance path per entity type. Example: for shifts, try
-- session -> roster -> division -> organisation.
CREATE TABLE cost_centre_inheritance_rules (
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    entity_type      text NOT NULL CHECK (char_length(entity_type) BETWEEN 1 AND 64),
    inherits_from    text NOT NULL CHECK (char_length(inherits_from) BETWEEN 1 AND 64),
    priority         integer NOT NULL CHECK (priority > 0),
    created_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, entity_type, inherits_from),
    CONSTRAINT cost_centre_inheritance_rules_priority_key
        UNIQUE (organisation_id, entity_type, priority),
    CONSTRAINT cost_centre_inheritance_rules_not_self
        CHECK (entity_type <> inherits_from)
);

COMMENT ON TABLE cost_centre_inheritance_rules IS
    'Configurable, ordered list of parent entity types consulted when a record has no direct cost centre.';

CREATE OR REPLACE FUNCTION carelibre_resolve_cost_centre(
    p_organisation_id uuid,
    p_entity_type text,
    p_entity_id uuid
) RETURNS uuid
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    v_type       text := p_entity_type;
    v_id         uuid := p_entity_id;
    v_cost_centre uuid;
    v_parent_type text;
    v_parent_id   uuid;
    v_depth       integer := 0;
BEGIN
    LOOP
        SELECT cca.cost_centre_id
        INTO v_cost_centre
        FROM cost_centre_assignments cca
        WHERE cca.organisation_id = p_organisation_id
          AND cca.entity_type = v_type
          AND cca.entity_id = v_id;

        IF v_cost_centre IS NOT NULL THEN
            RETURN v_cost_centre;
        END IF;

        SELECT rp.parent_type, rp.parent_id
        INTO v_parent_type, v_parent_id
        FROM cost_centre_inheritance_rules r
        JOIN record_parents rp
          ON rp.organisation_id = r.organisation_id
         AND rp.child_type = v_type
         AND rp.child_id = v_id
         AND rp.parent_type = r.inherits_from
        WHERE r.organisation_id = p_organisation_id
          AND r.entity_type = v_type
        ORDER BY r.priority
        LIMIT 1;

        IF v_parent_id IS NULL THEN
            RETURN NULL;
        END IF;

        v_depth := v_depth + 1;
        IF v_depth > 16 THEN
            RAISE EXCEPTION 'cost centre inheritance depth exceeded'
                USING ERRCODE = 'program_limit_exceeded';
        END IF;

        v_type := v_parent_type;
        v_id := v_parent_id;
    END LOOP;
END;
$$;

COMMENT ON FUNCTION carelibre_resolve_cost_centre(uuid, text, uuid) IS
    'Resolve a record''s effective cost centre: direct assignment first, then walk configured parent links.';

CREATE TRIGGER cost_centres_set_updated_at
    BEFORE UPDATE ON cost_centres
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER cost_centre_assignments_set_updated_at
    BEFORE UPDATE ON cost_centre_assignments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
