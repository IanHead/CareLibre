-- External organisations share the organisations table, but they are records in
-- a tenant rather than tenants in their own right. Two early triggers seeded
-- tenant-only data (permission personas and onboarding state) for every row,
-- including external counterparties.

CREATE OR REPLACE FUNCTION seed_permission_sets_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_permission_sets(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION create_onboarding_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        INSERT INTO organisation_onboarding (organisation_id)
        VALUES (NEW.id)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;

-- Remove the rows those triggers created for existing external organisations.
DELETE FROM organisation_onboarding
WHERE organisation_id IN (SELECT id FROM organisations WHERE tenant_id <> id);

ALTER TABLE permission_sets DISABLE TRIGGER permission_sets_guard_write;
ALTER TABLE permission_set_capabilities DISABLE TRIGGER permission_set_capabilities_guard;

DELETE FROM permission_sets
WHERE organisation_id IN (SELECT id FROM organisations WHERE tenant_id <> id);

ALTER TABLE permission_set_capabilities ENABLE TRIGGER permission_set_capabilities_guard;
ALTER TABLE permission_sets ENABLE TRIGGER permission_sets_guard_write;
