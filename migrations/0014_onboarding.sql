-- Onboarding state.
--
-- Task definitions live in code (`crate::onboarding`); only operator decisions
-- are persisted. Automatic tasks are detected on read and never written.

CREATE TABLE organisation_onboarding (
    organisation_id    uuid PRIMARY KEY REFERENCES organisations (id) ON DELETE CASCADE,
    mode               text NOT NULL DEFAULT 'guided'
                       CHECK (mode IN ('guided', 'migration')),
    catalogue_version  integer NOT NULL DEFAULT 1,
    started_at         timestamptz NOT NULL DEFAULT now(),
    completed_at       timestamptz,
    dismissed_at       timestamptz,
    updated_at         timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE organisation_onboarding IS
    'Per-organisation onboarding mode and completion. Task catalogue lives in code.';

CREATE TABLE onboarding_task_states (
    organisation_id  uuid NOT NULL REFERENCES organisation_onboarding (organisation_id) ON DELETE CASCADE,
    task_key         text NOT NULL CHECK (char_length(task_key) BETWEEN 1 AND 64),
    status           text NOT NULL CHECK (status IN ('done', 'skipped')),
    reason           text,
    marked_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    marked_at        timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, task_key)
);

COMMENT ON TABLE onboarding_task_states IS
    'Manual onboarding task decisions. No row means todo.';

CREATE TRIGGER organisation_onboarding_set_updated_at
    BEFORE UPDATE ON organisation_onboarding
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE OR REPLACE FUNCTION create_onboarding_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO organisation_onboarding (organisation_id)
    VALUES (NEW.id)
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_create_onboarding
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION create_onboarding_for_organisation();

INSERT INTO organisation_onboarding (organisation_id)
SELECT id FROM organisations
ON CONFLICT DO NOTHING;
