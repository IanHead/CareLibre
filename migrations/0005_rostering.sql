-- Rostering: shifts, sessions and client/worker assignments.
--
-- A shift may represent service delivery, a property's opening hours or a
-- respite program. Assignments may hang directly off a shift or off a session
-- inside it. Contacts are not typed, so the same contact can be a client on one
-- shift and a worker on another.

CREATE TABLE shifts (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    division_id      uuid,
    title            text,
    kind             text NOT NULL DEFAULT 'service_delivery'
                     CHECK (kind IN ('service_delivery', 'opening_hours', 'respite_program')),
    starts_at        timestamptz NOT NULL,
    ends_at          timestamptz NOT NULL,
    location         text,
    status           text NOT NULL DEFAULT 'draft'
                     CHECK (status IN ('draft', 'published', 'cancelled', 'completed')),
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT shifts_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT shifts_time_order CHECK (ends_at > starts_at),
    CONSTRAINT shifts_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id)
);

CREATE INDEX shifts_org_start_idx ON shifts (organisation_id, starts_at);
CREATE INDEX shifts_division_idx ON shifts (organisation_id, division_id);

COMMENT ON TABLE shifts IS
    'A rostered block of time. Can hold client/worker assignments directly or contain sessions. Also used for opening hours and respite programs.';
COMMENT ON COLUMN shifts.kind IS
    'service_delivery = care visit, opening_hours = property availability, respite_program = group/respite block.';

CREATE TABLE shift_sessions (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    shift_id         uuid NOT NULL,
    title            text,
    starts_at        timestamptz NOT NULL,
    ends_at          timestamptz NOT NULL,
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT shift_sessions_id_org_key UNIQUE (organisation_id, id),
    -- Redundant unique key so assignments can prove session and shift belong
    -- together with a single composite foreign key.
    CONSTRAINT shift_sessions_id_org_shift_key UNIQUE (organisation_id, shift_id, id),
    CONSTRAINT shift_sessions_time_order CHECK (ends_at > starts_at),
    CONSTRAINT shift_sessions_shift_fk
        FOREIGN KEY (organisation_id, shift_id)
        REFERENCES shifts (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX shift_sessions_shift_idx
    ON shift_sessions (organisation_id, shift_id, starts_at);

COMMENT ON TABLE shift_sessions IS
    'Sub-blocks inside a shift that can themselves hold client and worker assignments.';

CREATE TABLE client_assignments (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    shift_id         uuid NOT NULL,
    session_id       uuid,
    contact_id       uuid NOT NULL,
    status           text NOT NULL DEFAULT 'assigned'
                     CHECK (status IN ('assigned', 'cancelled', 'attended', 'no_show')),
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT client_assignments_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT client_assignments_shift_fk
        FOREIGN KEY (organisation_id, shift_id)
        REFERENCES shifts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_assignments_session_fk
        FOREIGN KEY (organisation_id, shift_id, session_id)
        REFERENCES shift_sessions (organisation_id, shift_id, id) ON DELETE CASCADE,
    CONSTRAINT client_assignments_contact_fk
        FOREIGN KEY (organisation_id, contact_id)
        REFERENCES contacts (organisation_id, id)
);

CREATE UNIQUE INDEX client_assignments_shift_unique
    ON client_assignments (organisation_id, shift_id, contact_id)
    WHERE session_id IS NULL;
CREATE UNIQUE INDEX client_assignments_session_unique
    ON client_assignments (organisation_id, session_id, contact_id)
    WHERE session_id IS NOT NULL;
CREATE INDEX client_assignments_contact_idx
    ON client_assignments (organisation_id, contact_id);

COMMENT ON TABLE client_assignments IS
    'A participant attending a shift or a session within it. Membership is per shift; the contact record itself is untyped.';

CREATE TABLE worker_assignments (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    shift_id         uuid NOT NULL,
    session_id       uuid,
    contact_id       uuid NOT NULL,
    status           text NOT NULL DEFAULT 'assigned'
                     CHECK (status IN ('assigned', 'cancelled', 'attended', 'no_show')),
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT worker_assignments_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT worker_assignments_shift_fk
        FOREIGN KEY (organisation_id, shift_id)
        REFERENCES shifts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT worker_assignments_session_fk
        FOREIGN KEY (organisation_id, shift_id, session_id)
        REFERENCES shift_sessions (organisation_id, shift_id, id) ON DELETE CASCADE,
    CONSTRAINT worker_assignments_contact_fk
        FOREIGN KEY (organisation_id, contact_id)
        REFERENCES contacts (organisation_id, id)
);

CREATE UNIQUE INDEX worker_assignments_shift_unique
    ON worker_assignments (organisation_id, shift_id, contact_id)
    WHERE session_id IS NULL;
CREATE UNIQUE INDEX worker_assignments_session_unique
    ON worker_assignments (organisation_id, session_id, contact_id)
    WHERE session_id IS NOT NULL;
CREATE INDEX worker_assignments_contact_idx
    ON worker_assignments (organisation_id, contact_id);

COMMENT ON TABLE worker_assignments IS
    'A worker delivering a shift or session. Assignment is per shift; the contact record itself is untyped.';

CREATE TRIGGER shifts_set_updated_at
    BEFORE UPDATE ON shifts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER shift_sessions_set_updated_at
    BEFORE UPDATE ON shift_sessions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER client_assignments_set_updated_at
    BEFORE UPDATE ON client_assignments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER worker_assignments_set_updated_at
    BEFORE UPDATE ON worker_assignments
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
