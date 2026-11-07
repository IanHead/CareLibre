-- Rostering suitability: shift context, explicit constraints and observed history.
--
-- Declared care relationships stay in `relationships`. This migration adds the
-- separate evidence needed to recommend workers:
--
--   * physical sites, and a service/site on every shift;
--   * explicit preferred/avoid/blocked constraints;
--   * assignment outcomes and ratings;
--   * history views that aggregate what actually happened.
--
-- Scores are derived from these views on demand rather than stored on edges, so
-- every recommendation can be explained and recomputed.

-- ---------------------------------------------------------------------------
-- 1. Sites and shift context
-- ---------------------------------------------------------------------------

CREATE TABLE sites (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    division_id      uuid,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    address          text,
    notes            text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT sites_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT sites_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id)
);

CREATE UNIQUE INDEX sites_name_key
    ON sites (organisation_id, lower(name));
CREATE INDEX sites_division_idx ON sites (organisation_id, division_id);

COMMENT ON TABLE sites IS
    'A physical place where care happens: a client home, day program, respite house or office. A shift may name one site and one service.';

ALTER TABLE shifts
    ADD COLUMN service_id uuid,
    ADD COLUMN site_id uuid,
    ADD CONSTRAINT shifts_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id) ON DELETE SET NULL (service_id),
    ADD CONSTRAINT shifts_site_fk
        FOREIGN KEY (organisation_id, site_id)
        REFERENCES sites (organisation_id, id) ON DELETE SET NULL (site_id);

CREATE INDEX shifts_service_idx ON shifts (organisation_id, service_id);
CREATE INDEX shifts_site_idx ON shifts (organisation_id, site_id);

COMMENT ON COLUMN shifts.service_id IS
    'Billable service delivered by this shift. Suitability can learn worker familiarity with the service.';
COMMENT ON COLUMN shifts.site_id IS
    'Physical site or program location. Suitability can learn worker familiarity with the site.';

-- ---------------------------------------------------------------------------
-- 2. Explicit rostering constraints
-- ---------------------------------------------------------------------------

CREATE TABLE roster_constraints (
    id                   uuid PRIMARY KEY,
    organisation_id      uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    worker_contact_id    uuid NOT NULL,
    target_contact_id    uuid,
    target_site_id       uuid,
    target_service_id    uuid,
    target_division_id   uuid,
    kind                 text NOT NULL CHECK (kind IN ('preferred', 'avoid', 'blocked')),
    strength             smallint NOT NULL DEFAULT 3 CHECK (strength BETWEEN 1 AND 5),
    reason               text,
    source               text NOT NULL DEFAULT 'manual',
    is_active            boolean NOT NULL DEFAULT true,
    start_date           date,
    end_date             date,
    created_by           uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at           timestamptz NOT NULL DEFAULT now(),
    updated_at           timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT roster_constraints_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT roster_constraints_one_target CHECK (
        num_nonnulls(target_contact_id, target_site_id, target_service_id, target_division_id) = 1
    ),
    CONSTRAINT roster_constraints_not_self CHECK (
        target_contact_id IS NULL OR target_contact_id <> worker_contact_id
    ),
    CONSTRAINT roster_constraints_dates CHECK (
        end_date IS NULL OR start_date IS NULL OR end_date >= start_date
    ),
    CONSTRAINT roster_constraints_worker_fk
        FOREIGN KEY (organisation_id, worker_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT roster_constraints_target_contact_fk
        FOREIGN KEY (organisation_id, target_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT roster_constraints_target_site_fk
        FOREIGN KEY (organisation_id, target_site_id)
        REFERENCES sites (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT roster_constraints_target_service_fk
        FOREIGN KEY (organisation_id, target_service_id)
        REFERENCES services (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT roster_constraints_target_division_fk
        FOREIGN KEY (organisation_id, target_division_id)
        REFERENCES divisions (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT roster_constraints_unique UNIQUE NULLS NOT DISTINCT (
        organisation_id, worker_contact_id, target_contact_id, target_site_id,
        target_service_id, target_division_id, kind
    )
);

CREATE INDEX roster_constraints_worker_idx
    ON roster_constraints (organisation_id, worker_contact_id) WHERE is_active;
CREATE INDEX roster_constraints_contact_idx
    ON roster_constraints (organisation_id, target_contact_id)
    WHERE is_active AND target_contact_id IS NOT NULL;
CREATE INDEX roster_constraints_site_idx
    ON roster_constraints (organisation_id, target_site_id)
    WHERE is_active AND target_site_id IS NOT NULL;
CREATE INDEX roster_constraints_service_idx
    ON roster_constraints (organisation_id, target_service_id)
    WHERE is_active AND target_service_id IS NOT NULL;
CREATE INDEX roster_constraints_division_idx
    ON roster_constraints (organisation_id, target_division_id)
    WHERE is_active AND target_division_id IS NOT NULL;

COMMENT ON TABLE roster_constraints IS
    'Explicit rostering preferences and restrictions. preferred = bonus, avoid = penalty, blocked = hard exclusion. Contact targets are symmetric: a conflict between two workers applies in both directions.';
COMMENT ON COLUMN roster_constraints.strength IS
    'How strongly the constraint applies, 1 (weak) to 5 (strong). A blocked constraint is always hard regardless of strength.';
COMMENT ON COLUMN roster_constraints.source IS
    'Where the constraint came from: manual, client_request, incident, hr, import. Free text.';

CREATE TRIGGER roster_constraints_set_updated_at
    BEFORE UPDATE ON roster_constraints
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER sites_set_updated_at
    BEFORE UPDATE ON sites
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- 3. Assignment outcomes
-- ---------------------------------------------------------------------------

ALTER TABLE worker_assignments
    ADD COLUMN outcome text CHECK (outcome IN ('positive', 'neutral', 'negative')),
    ADD COLUMN outcome_rating smallint CHECK (outcome_rating BETWEEN 1 AND 5);

COMMENT ON COLUMN worker_assignments.outcome IS
    'Optional assessment of how the assignment went. Feeds suitability history; never edits the contact or relationship record.';
COMMENT ON COLUMN worker_assignments.outcome_rating IS
    'Optional 1-5 rating of the assignment outcome.';

-- ---------------------------------------------------------------------------
-- 4. Derived history views
-- ---------------------------------------------------------------------------

CREATE VIEW worker_client_history AS
SELECT
    w.organisation_id,
    w.contact_id AS worker_contact_id,
    c.contact_id AS client_contact_id,
    count(DISTINCT s.id) FILTER (WHERE w.status = 'attended') AS attended_shifts,
    count(DISTINCT s.id) FILTER (WHERE w.status = 'no_show') AS no_show_shifts,
    count(DISTINCT s.id) FILTER (WHERE w.status = 'cancelled') AS cancelled_shifts,
    max(s.starts_at) FILTER (WHERE w.status = 'attended') AS last_worked_at,
    CAST(avg(w.outcome_rating) FILTER (WHERE w.outcome_rating IS NOT NULL) AS double precision) AS average_rating,
    count(*) FILTER (WHERE w.outcome = 'positive') AS positive_outcomes,
    count(*) FILTER (WHERE w.outcome = 'negative') AS negative_outcomes
FROM worker_assignments w
JOIN client_assignments c
  ON c.organisation_id = w.organisation_id
 AND c.shift_id = w.shift_id
 AND (w.session_id IS NULL OR c.session_id IS NULL OR w.session_id = c.session_id)
JOIN shifts s
  ON s.organisation_id = w.organisation_id
 AND s.id = w.shift_id
GROUP BY w.organisation_id, w.contact_id, c.contact_id;

COMMENT ON VIEW worker_client_history IS
    'Observed worker/client history: attended shifts, no-shows, cancellations, recency and outcome ratings.';

CREATE VIEW worker_worker_history AS
SELECT
    a.organisation_id,
    a.contact_id AS worker_a_id,
    b.contact_id AS worker_b_id,
    count(DISTINCT s.id) FILTER (
        WHERE a.status = 'attended' AND b.status = 'attended'
    ) AS attended_shifts,
    count(DISTINCT s.id) FILTER (
        WHERE a.status = 'no_show' OR b.status = 'no_show'
    ) AS no_show_shifts,
    max(s.starts_at) FILTER (
        WHERE a.status = 'attended' AND b.status = 'attended'
    ) AS last_worked_at
FROM worker_assignments a
JOIN worker_assignments b
  ON b.organisation_id = a.organisation_id
 AND b.shift_id = a.shift_id
 AND a.contact_id < b.contact_id
 AND (a.session_id IS NULL OR b.session_id IS NULL OR a.session_id = b.session_id)
JOIN shifts s
  ON s.organisation_id = a.organisation_id
 AND s.id = a.shift_id
GROUP BY a.organisation_id, a.contact_id, b.contact_id;

COMMENT ON VIEW worker_worker_history IS
    'Observed co-worker history: how often two workers attended the same shift.';

CREATE VIEW worker_context_history AS
SELECT organisation_id, worker_contact_id, 'site'::text AS context_kind,
       site_id AS context_id, attended_shifts, last_worked_at
FROM (
    SELECT w.organisation_id, w.contact_id AS worker_contact_id, s.site_id,
           count(DISTINCT s.id) FILTER (WHERE w.status = 'attended') AS attended_shifts,
           max(s.starts_at) FILTER (WHERE w.status = 'attended') AS last_worked_at
    FROM worker_assignments w
    JOIN shifts s ON s.organisation_id = w.organisation_id AND s.id = w.shift_id
    WHERE s.site_id IS NOT NULL
    GROUP BY w.organisation_id, w.contact_id, s.site_id
) site_history
UNION ALL
SELECT organisation_id, worker_contact_id, 'service'::text,
       service_id, attended_shifts, last_worked_at
FROM (
    SELECT w.organisation_id, w.contact_id AS worker_contact_id, s.service_id,
           count(DISTINCT s.id) FILTER (WHERE w.status = 'attended') AS attended_shifts,
           max(s.starts_at) FILTER (WHERE w.status = 'attended') AS last_worked_at
    FROM worker_assignments w
    JOIN shifts s ON s.organisation_id = w.organisation_id AND s.id = w.shift_id
    WHERE s.service_id IS NOT NULL
    GROUP BY w.organisation_id, w.contact_id, s.service_id
) service_history
UNION ALL
SELECT organisation_id, worker_contact_id, 'division'::text,
       division_id, attended_shifts, last_worked_at
FROM (
    SELECT w.organisation_id, w.contact_id AS worker_contact_id, s.division_id,
           count(DISTINCT s.id) FILTER (WHERE w.status = 'attended') AS attended_shifts,
           max(s.starts_at) FILTER (WHERE w.status = 'attended') AS last_worked_at
    FROM worker_assignments w
    JOIN shifts s ON s.organisation_id = w.organisation_id AND s.id = w.shift_id
    WHERE s.division_id IS NOT NULL
    GROUP BY w.organisation_id, w.contact_id, s.division_id
) division_history;

COMMENT ON VIEW worker_context_history IS
    'Observed worker familiarity with a site, service or division, keyed by context_kind/context_id.';
