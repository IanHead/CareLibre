-- Allow organisation deletion to cascade through organisation_users.
--
-- The last-owner guard must protect membership changes, but not block deleting
-- the tenant itself. During the cascade the organisation row is already gone, so
-- skip the check in that case.

CREATE OR REPLACE FUNCTION guard_last_owner() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_organisation uuid := COALESCE(NEW.organisation_id, OLD.organisation_id);
BEGIN
    IF NOT EXISTS (SELECT 1 FROM organisations WHERE id = v_organisation) THEN
        RETURN COALESCE(NEW, OLD);
    END IF;

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
