-- Relationship lifecycle and history.
--
-- Relationships change over time: they end (a worker is offboarded, a GP
-- leaves the area), pass to another party (a new support coordinator), change
-- role (family member becomes guardian), get corrected, or turn out to have
-- been entered in error. Instead of deleting rows, every change is recorded:
--
-- * `relationships.end_reason` explains every end date. A relationship can end
--   in the future, e.g. at the end of a notice period.
-- * `relationships.previous_id` links a relationship to the one it replaced,
--   so a transfer or role change reads as one continuous history.
-- * `relationships.removed_*` hides a relationship recorded in error from every
--   view while keeping it for the audit trail. Only `relationships.remove`
--   may do this (administrators by default).
-- * `relationship_events` is an append-only log of every action, with who did
--   it, when, why, and the values before and after.

-- ---------------------------------------------------------------------------
-- Capabilities
-- ---------------------------------------------------------------------------

INSERT INTO capabilities (key, category, label, description, sort_order) VALUES
    ('relationships.remove', 'People', 'Remove relationships',
     'Remove relationships recorded in error. They stay in the audit trail.', 16),
    ('relationships.incident_notes', 'People', 'View incident notes',
     'Read the notes on relationship changes recorded as incidents.', 17);

-- The administrator set holds every capability and is otherwise immutable.
ALTER TABLE permission_set_capabilities DISABLE TRIGGER permission_set_capabilities_guard;

INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, c.key
FROM permission_sets ps
CROSS JOIN (VALUES ('relationships.remove'), ('relationships.incident_notes')) AS c (key)
WHERE ps.persona_key = 'administrator';

ALTER TABLE permission_set_capabilities ENABLE TRIGGER permission_set_capabilities_guard;

-- Managers read incident notes by default.
INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, 'relationships.incident_notes'
FROM permission_sets ps
WHERE ps.persona_key IN ('senior_management', 'house_manager');

-- New organisations: the administrator set already copies the whole catalogue,
-- so only the manager personas need the default added as they are created.
CREATE OR REPLACE FUNCTION grant_default_relationship_capabilities() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.persona_key IN ('senior_management', 'house_manager') THEN
        INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
        VALUES (NEW.organisation_id, NEW.id, 'relationships.incident_notes');
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER permission_sets_default_relationship_capabilities
    AFTER INSERT ON permission_sets
    FOR EACH ROW EXECUTE FUNCTION grant_default_relationship_capabilities();

-- ---------------------------------------------------------------------------
-- Relationship columns
-- ---------------------------------------------------------------------------

ALTER TABLE relationships
    ADD COLUMN end_reason    text,
    ADD COLUMN previous_id   uuid,
    ADD COLUMN removed_at    timestamptz,
    ADD COLUMN removed_by    uuid REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN remove_reason text;

-- Relationships that already ended carry no reason; record them as "other".
UPDATE relationships SET end_reason = 'other' WHERE end_date IS NOT NULL;

ALTER TABLE relationships
    ADD CONSTRAINT relationships_end_reason_valid CHECK (
        end_reason IS NULL OR end_reason IN (
            'offboarded', 'moved_away', 'plan_ended', 'deceased',
            'changed_provider', 'circumstances_changed', 'incident', 'other'
        )
    ),
    ADD CONSTRAINT relationships_end_needs_reason CHECK (
        (end_date IS NULL) = (end_reason IS NULL)
    ),
    ADD CONSTRAINT relationships_removed_needs_reason CHECK (
        removed_at IS NULL OR char_length(btrim(remove_reason)) > 0
    );

CREATE INDEX relationships_previous_idx
    ON relationships (organisation_id, previous_id)
    WHERE previous_id IS NOT NULL;

COMMENT ON COLUMN relationships.end_reason IS
    'Why the relationship ends. Required whenever end_date is set.';
COMMENT ON COLUMN relationships.previous_id IS
    'The relationship this one replaced through a transfer or role change. Not a foreign key, so the link survives if the earlier row is ever deleted with its contact.';
COMMENT ON COLUMN relationships.removed_at IS
    'Set when the relationship was recorded in error. Removed relationships are hidden from every view but kept for the audit trail.';

-- Removed relationships no longer count anywhere.
CREATE OR REPLACE VIEW relationship_directions AS
SELECT
    r.organisation_id,
    r.id AS relationship_id,
    'from'::text AS side,
    CASE WHEN r.from_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END
        AS subject_kind,
    COALESCE(r.from_contact_id, r.from_organisation_id) AS subject_id,
    r.from_role AS subject_role,
    CASE WHEN r.to_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END
        AS counterpart_kind,
    COALESCE(r.to_contact_id, r.to_organisation_id) AS counterpart_id,
    r.to_role AS counterpart_role,
    r.start_date,
    r.end_date,
    r.notes
FROM relationships r
WHERE r.removed_at IS NULL
UNION ALL
SELECT
    r.organisation_id,
    r.id,
    'to',
    CASE WHEN r.to_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END,
    COALESCE(r.to_contact_id, r.to_organisation_id),
    r.to_role,
    CASE WHEN r.from_contact_id IS NOT NULL THEN 'contact' ELSE 'organisation' END,
    COALESCE(r.from_contact_id, r.from_organisation_id),
    r.from_role,
    r.start_date,
    r.end_date,
    r.notes
FROM relationships r
WHERE r.removed_at IS NULL;

CREATE OR REPLACE VIEW contact_care_roles AS
SELECT
    c.id AS contact_id,
    EXISTS (
        SELECT 1
        FROM relationships r
        WHERE r.organisation_id = c.organisation_id
          AND r.removed_at IS NULL
          AND (r.end_date IS NULL OR r.end_date >= CURRENT_DATE)
          AND (
              (r.from_contact_id = c.id
               AND r.to_organisation_id = c.organisation_id
               AND r.from_role = 'care_recipient' AND r.to_role = 'provider')
              OR
              (r.to_contact_id = c.id
               AND r.from_organisation_id = c.organisation_id
               AND r.to_role = 'care_recipient' AND r.from_role = 'provider')
          )
    ) AS is_client,
    EXISTS (
        SELECT 1
        FROM relationships r
        WHERE r.organisation_id = c.organisation_id
          AND r.removed_at IS NULL
          AND (r.end_date IS NULL OR r.end_date >= CURRENT_DATE)
          AND (
              (r.from_contact_id = c.id
               AND r.to_organisation_id = c.organisation_id
               AND r.from_role = 'employee' AND r.to_role = 'employer')
              OR
              (r.to_contact_id = c.id
               AND r.from_organisation_id = c.organisation_id
               AND r.to_role = 'employee' AND r.from_role = 'employer')
          )
    ) AS is_worker
FROM contacts c;

-- ---------------------------------------------------------------------------
-- History
-- ---------------------------------------------------------------------------

CREATE TABLE relationship_events (
    id                       uuid PRIMARY KEY,
    organisation_id          uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    relationship_id          uuid NOT NULL,
    action                   text NOT NULL CHECK (action IN (
        'added', 'ended', 'transferred', 'role_changed', 'corrected', 'removed'
    )),
    effective_date           date,
    reason                   text,
    note                     text,
    before                   jsonb,
    after                    jsonb,
    related_relationship_id  uuid,
    actor_user_id            uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at               timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX relationship_events_relationship_idx
    ON relationship_events (organisation_id, relationship_id, created_at);

COMMENT ON TABLE relationship_events IS
    'Append-only history of every relationship change: who, when, why, and the values before and after. relationship_id is deliberately not a foreign key so history outlives the row.';

-- History is append-only. Rows go only when their organisation is deleted.
CREATE OR REPLACE FUNCTION guard_relationship_events() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND NOT EXISTS (SELECT 1 FROM organisations WHERE id = OLD.organisation_id)
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'relationship history is append-only'
        USING ERRCODE = 'object_not_in_prerequisite_state';
END;
$$;

CREATE TRIGGER relationship_events_append_only
    BEFORE UPDATE OR DELETE ON relationship_events
    FOR EACH ROW EXECUTE FUNCTION guard_relationship_events();

-- Every new relationship, from any writer (API, import, seed), starts its
-- history with an "added" event. The API names the acting user for the
-- transaction with set_config('carelibre.actor_user_id', ..., true).
CREATE OR REPLACE FUNCTION record_relationship_added() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_actor text := NULLIF(current_setting('carelibre.actor_user_id', true), '');
BEGIN
    INSERT INTO relationship_events
        (id, organisation_id, relationship_id, action, effective_date, reason, note,
         after, related_relationship_id, actor_user_id)
    VALUES
        (gen_random_uuid(), NEW.organisation_id, NEW.id, 'added', NEW.start_date,
         NULLIF(current_setting('carelibre.event_reason', true), ''),
         NULLIF(current_setting('carelibre.event_note', true), ''),
         jsonb_build_object(
             'from_role', NEW.from_role, 'to_role', NEW.to_role,
             'start_date', NEW.start_date, 'end_date', NEW.end_date,
             'end_reason', NEW.end_reason, 'notes', NEW.notes
         ),
         NEW.previous_id, v_actor::uuid);
    RETURN NEW;
END;
$$;

CREATE TRIGGER relationships_record_added
    AFTER INSERT ON relationships
    FOR EACH ROW EXECUTE FUNCTION record_relationship_added();

-- Give every existing relationship a starting point in its history.
INSERT INTO relationship_events
    (id, organisation_id, relationship_id, action, effective_date, note, created_at)
SELECT gen_random_uuid(), r.organisation_id, r.id, 'added', r.start_date,
       'Recorded before relationship history was kept.', r.created_at
FROM relationships r;

INSERT INTO relationship_events
    (id, organisation_id, relationship_id, action, effective_date, reason, note, created_at)
SELECT gen_random_uuid(), r.organisation_id, r.id, 'ended', r.end_date, r.end_reason,
       'Recorded before relationship history was kept.', r.updated_at
FROM relationships r
WHERE r.end_date IS NOT NULL;
