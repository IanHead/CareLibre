-- Organisation branding for the desktop/mobile clients.
--
-- The organisation name already lives on `organisations`. The logo is stored in
-- a separate table so counterpart organisations (external providers etc.) do not
-- carry image blobs. Only the tenant root has branding.
--
-- Logos are served from the API with a stable SHA-256 ETag, so clients can cache
-- them and revalidate cheaply.

CREATE TABLE organisation_branding (
    organisation_id    uuid PRIMARY KEY REFERENCES organisations (id) ON DELETE CASCADE,
    logo               bytea NOT NULL,
    logo_content_type  text NOT NULL CHECK (
        logo_content_type IN ('image/png', 'image/jpeg', 'image/webp', 'image/gif')
    ),
    logo_filename      text CHECK (
        logo_filename IS NULL OR char_length(logo_filename) BETWEEN 1 AND 255
    ),
    logo_sha256        text NOT NULL CHECK (char_length(logo_sha256) = 64),
    updated_by         uuid REFERENCES users (id) ON DELETE SET NULL,
    updated_at         timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE organisation_branding IS
    'Tenant branding used by clients: logo bytes, content type and a SHA-256 ETag. The organisation name remains on organisations.';
COMMENT ON COLUMN organisation_branding.logo_sha256 IS
    'Lowercase hex SHA-256 of the logo bytes, used as the HTTP ETag.';

-- Branding belongs to tenant roots only, never to external counterparties.
CREATE OR REPLACE FUNCTION guard_organisation_branding_tenant() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM organisations
        WHERE id = NEW.organisation_id AND tenant_id = id
    ) THEN
        RAISE EXCEPTION 'branding may only be attached to a tenant root organisation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisation_branding_tenant_guard
    BEFORE INSERT OR UPDATE ON organisation_branding
    FOR EACH ROW EXECUTE FUNCTION guard_organisation_branding_tenant();
