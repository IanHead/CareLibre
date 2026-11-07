-- Make relationships explicitly two-sided.
--
-- A relationship is one row with a role for each endpoint. Reading the row from
-- left to right uses `from_role`; reading it from right to left uses `to_role`.
-- For example, a doctor and their patient is stored once as:
--
--     from_role = 'general_practitioner', to_role = 'patient'
--
-- Both labels are required, so a connection can always be rendered from either
-- side and the relationship graph never has to guess an inverse.

ALTER TABLE relationships
    RENAME COLUMN relationship_type TO from_role;

ALTER TABLE relationships
    RENAME COLUMN inverse_name TO to_role;

-- Existing rows either already carried an inverse label or only had the forward
-- direction. Fall back to the forward label so the column can become required;
-- the demo seed and the CSV importer now always supply both.
UPDATE relationships
SET to_role = from_role
WHERE to_role IS NULL OR btrim(to_role) = '';

ALTER TABLE relationships
    ALTER COLUMN to_role SET NOT NULL,
    ADD CONSTRAINT relationships_to_role_check
        CHECK (char_length(to_role) BETWEEN 1 AND 64);

COMMENT ON COLUMN relationships.from_role IS
    'Role of the from endpoint, read as "<from> is <from_role> of <to>". Free text, e.g. general_practitioner, guardian, employee.';
COMMENT ON COLUMN relationships.to_role IS
    'Role of the to endpoint, read as "<to> is <to_role> of <from>". Free text, e.g. patient, person_supported, employer.';
COMMENT ON TABLE relationships IS
    'Typed two-sided links between contacts and/or organisations. Each endpoint carries its own role (doctor/patient, guardian/person supported, employee/employer), so the graph can be traversed in either direction.';

-- Flatten each relationship into two directed rows: one from each endpoint's
-- point of view. This is the shape a graph client wants: a subject, the role
-- they hold, and the counterpart's role in return.
CREATE VIEW relationship_directions AS
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
FROM relationships r;

COMMENT ON VIEW relationship_directions IS
    'Each relationship expanded into both directions. `subject` holds `subject_role` and the other endpoint holds `counterpart_role`, e.g. doctor/patient or patient/doctor.';
