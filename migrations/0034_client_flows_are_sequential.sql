-- Client intake paths follow a real eligibility and onboarding sequence.
--
-- Worker, other-contact and organisation flows keep the current free-movement
-- default; admins can still change any flow explicitly.

UPDATE intake_flows
SET stage_movement = 'sequential'
WHERE subject_kind = 'client';

CREATE OR REPLACE FUNCTION seed_intake_flows_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_intake_flows(NEW.id);
        UPDATE intake_flows
        SET stage_movement = 'sequential'
        WHERE organisation_id = NEW.id AND subject_kind = 'client';
    END IF;
    RETURN NEW;
END;
$$;
