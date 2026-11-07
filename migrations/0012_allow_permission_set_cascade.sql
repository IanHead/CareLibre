-- Permission-set guards must not block deleting the tenant they belong to.
--
-- During an organisation cascade the parent rows are already gone. Skip the
-- immutability checks when the organisation (or the set itself) no longer
-- exists; normal edits and deletes inside a live tenant are still guarded.

CREATE OR REPLACE FUNCTION guard_permission_set_write() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_organisation uuid := COALESCE(NEW.organisation_id, OLD.organisation_id);
BEGIN
    IF NOT EXISTS (SELECT 1 FROM organisations WHERE id = v_organisation) THEN
        RETURN COALESCE(NEW, OLD);
    END IF;

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

CREATE OR REPLACE FUNCTION guard_permission_set_capabilities() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_set uuid := COALESCE(NEW.permission_set_id, OLD.permission_set_id);
    v_organisation uuid;
BEGIN
    SELECT organisation_id INTO v_organisation FROM permission_sets WHERE id = v_set;

    IF v_organisation IS NULL
       OR NOT EXISTS (SELECT 1 FROM organisations WHERE id = v_organisation)
    THEN
        RETURN COALESCE(NEW, OLD);
    END IF;

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
