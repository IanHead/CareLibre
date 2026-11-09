-- Clients and workers are relationships with the organisation.
--
-- A contact is a client of the organisation when a current relationship has the
-- contact as `care_recipient` and the organisation (the tenant root) as
-- `provider`. A contact is a worker when a current relationship has the contact
-- as `employee` and the organisation as `employer`. Either direction counts,
-- because relationships are two-sided.
--
-- This replaces the `details.contact_role` hint from 0010, which is migrated
-- into relationships below and then removed.

CREATE VIEW contact_care_roles AS
SELECT
    c.id AS contact_id,
    EXISTS (
        SELECT 1
        FROM relationships r
        WHERE r.organisation_id = c.organisation_id
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

COMMENT ON VIEW contact_care_roles IS
    'Whether each contact is currently a client (care_recipient of the organisation as provider) and/or a worker (employee of the organisation as employer). Derived from relationships; there is no stored contact type.';

-- Carry existing hints over as relationships, skipping any that already exist.
INSERT INTO relationships
    (id, organisation_id, from_contact_id, to_organisation_id, from_role, to_role)
SELECT gen_random_uuid(), c.organisation_id, c.id, c.organisation_id,
       'care_recipient', 'provider'
FROM contacts c
JOIN contact_care_roles roles ON roles.contact_id = c.id
WHERE c.details->>'contact_role' IN ('client', 'dual')
  AND NOT roles.is_client;

INSERT INTO relationships
    (id, organisation_id, from_contact_id, to_organisation_id, from_role, to_role)
SELECT gen_random_uuid(), c.organisation_id, c.id, c.organisation_id,
       'employee', 'employer'
FROM contacts c
JOIN contact_care_roles roles ON roles.contact_id = c.id
WHERE c.details->>'contact_role' IN ('worker', 'dual')
  AND NOT roles.is_worker;

DROP INDEX IF EXISTS contacts_org_role_idx;

UPDATE contacts
SET details = details - 'contact_role'
WHERE details ? 'contact_role';
