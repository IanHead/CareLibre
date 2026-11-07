-- Free-form details for contacts.
--
-- Contacts stay untyped: client/worker/doctor/family are relationship roles.
-- This JSONB column carries intake profile data (address, health needs,
-- employment terms, etc.) without turning every variant into schema. It is
-- also where the desktop stores the optional `contact_role` hint for views.

ALTER TABLE contacts
    ADD COLUMN details jsonb NOT NULL DEFAULT '{}'::jsonb;

CREATE INDEX contacts_org_role_idx
    ON contacts (organisation_id, (details->>'contact_role'));

COMMENT ON COLUMN contacts.details IS
    'Extended intake/profile data as JSONB. Untyped by design; custom fields are stored separately in custom_field_values.';
