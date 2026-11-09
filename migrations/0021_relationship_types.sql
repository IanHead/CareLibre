-- Relationship types: the catalogue of allowed role pairs.
--
-- Roles used to be free text, which invites near-duplicates ("brother" and
-- "brotherr", "doctor" and "GP"). Each organisation now keeps a catalogue of
-- role pairs, e.g. patient/general_practitioner or mother/son. New and changed
-- relationships must use an active pair, in either direction. Administrators
-- (`relationship_types.manage`) add pairs and retire ones no longer wanted;
-- retired pairs stay valid on relationships that already use them.

CREATE TABLE relationship_types (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    from_role        text NOT NULL CHECK (from_role ~ '^[a-z][a-z0-9_]{0,63}$'),
    to_role          text NOT NULL CHECK (to_role ~ '^[a-z][a-z0-9_]{0,63}$'),
    category         text NOT NULL DEFAULT 'other' CHECK (category IN (
        'care', 'family', 'health', 'services', 'work', 'other'
    )),
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now()
);

-- One entry per pair, whichever way round it was written.
CREATE UNIQUE INDEX relationship_types_pair_key
    ON relationship_types (organisation_id, LEAST(from_role, to_role), GREATEST(from_role, to_role));

CREATE TRIGGER relationship_types_set_updated_at
    BEFORE UPDATE ON relationship_types
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE relationship_types IS
    'Allowed relationship role pairs per organisation. A relationship may use a pair in either direction. Retired pairs (is_active = false) cannot be used for new relationships or role changes.';

-- The standard catalogue every organisation starts with.
CREATE OR REPLACE FUNCTION carelibre_seed_relationship_types(p_organisation_id uuid)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO relationship_types (id, organisation_id, from_role, to_role, category)
    SELECT gen_random_uuid(), p_organisation_id, pair.from_role, pair.to_role, pair.category
    FROM (
        VALUES
            ('care_recipient', 'provider', 'care'),
            ('participant', 'support_coordinator', 'care'),
            ('participant', 'plan_manager', 'care'),
            ('person_supported', 'emergency_contact', 'care'),
            ('person_supported', 'next_of_kin', 'care'),
            ('person_supported', 'primary_contact', 'care'),
            ('person_supported', 'representative', 'care'),
            ('person_supported', 'guardian', 'care'),
            ('person_supported', 'family', 'care'),
            ('person_supported', 'friend', 'care'),
            ('person_supported', 'other', 'care'),
            ('represented_person', 'guardian', 'care'),
            ('principal', 'enduring_power_of_attorney', 'care'),
            ('housemate', 'housemate', 'care'),
            ('patient', 'general_practitioner', 'health'),
            ('patient', 'paediatrician', 'health'),
            ('patient', 'geriatrician', 'health'),
            ('patient', 'psychiatrist', 'health'),
            ('patient', 'neurologist', 'health'),
            ('general_practitioner', 'practice', 'health'),
            ('paediatrician', 'practice', 'health'),
            ('geriatrician', 'practice', 'health'),
            ('psychiatrist', 'practice', 'health'),
            ('neurologist', 'practice', 'health'),
            ('client', 'occupational_therapy', 'services'),
            ('client', 'physiotherapy', 'services'),
            ('client', 'speech_pathology', 'services'),
            ('customer', 'pharmacy', 'services'),
            ('representative', 'organisation', 'services'),
            ('employee', 'employer', 'work'),
            ('partner', 'partner', 'family'),
            ('cousin', 'cousin', 'family')
    ) AS pair (from_role, to_role, category)
    UNION ALL
    -- Gendered family roles, every combination.
    SELECT gen_random_uuid(), p_organisation_id, kin.a_role, kin.b_role, 'family'
    FROM (VALUES
            ('mother', 'son'), ('mother', 'daughter'),
            ('father', 'son'), ('father', 'daughter'),
            ('grandmother', 'grandson'), ('grandmother', 'granddaughter'),
            ('grandfather', 'grandson'), ('grandfather', 'granddaughter'),
            ('aunt', 'niece'), ('aunt', 'nephew'),
            ('uncle', 'niece'), ('uncle', 'nephew'),
            ('brother', 'brother'), ('brother', 'sister'), ('sister', 'sister'),
            ('husband', 'wife'), ('husband', 'husband'), ('wife', 'wife')
         ) AS kin (a_role, b_role)
    ON CONFLICT DO NOTHING;
$$;

-- New tenant organisations start with the standard catalogue.
CREATE OR REPLACE FUNCTION seed_relationship_types_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_relationship_types(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_seed_relationship_types
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION seed_relationship_types_for_organisation();

-- Existing tenants: the standard catalogue, plus every pair already in use so
-- nothing that exists today becomes invalid.
SELECT carelibre_seed_relationship_types(id) FROM organisations WHERE tenant_id = id;

INSERT INTO relationship_types (id, organisation_id, from_role, to_role, category)
SELECT gen_random_uuid(), used.organisation_id, used.from_role, used.to_role, 'other'
FROM (
    SELECT DISTINCT ON (organisation_id, LEAST(from_role, to_role), GREATEST(from_role, to_role))
           organisation_id, from_role, to_role
    FROM relationships
    WHERE removed_at IS NULL
      AND from_role ~ '^[a-z][a-z0-9_]{0,63}$'
      AND to_role ~ '^[a-z][a-z0-9_]{0,63}$'
    ORDER BY organisation_id, LEAST(from_role, to_role), GREATEST(from_role, to_role)
) AS used
ON CONFLICT DO NOTHING;

-- Who may manage the catalogue: administrators by default.
INSERT INTO capabilities (key, category, label, description, sort_order) VALUES
    ('relationship_types.manage', 'People', 'Manage relationship types',
     'Add relationship role pairs and retire ones no longer used.', 18);

ALTER TABLE permission_set_capabilities DISABLE TRIGGER permission_set_capabilities_guard;
INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, 'relationship_types.manage'
FROM permission_sets ps
WHERE ps.persona_key = 'administrator';
ALTER TABLE permission_set_capabilities ENABLE TRIGGER permission_set_capabilities_guard;
