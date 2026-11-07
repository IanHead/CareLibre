-- CareLibre initial schema.
--
-- Foundation tables only: organisations (tenants), users, memberships,
-- authentication sessions and an audit log. Domain tables (divisions, contacts,
-- services, rates, rostering) arrive in later migrations.

CREATE TABLE organisations (
    id          uuid PRIMARY KEY,
    slug        text NOT NULL,
    name        text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 120),
    edition     text NOT NULL DEFAULT 'community'
                CHECK (edition IN ('community', 'business', 'enterprise')),
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT organisations_slug_key UNIQUE (slug),
    CONSTRAINT organisations_slug_format
        CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{1,61}[a-z0-9])$')
);

CREATE TABLE users (
    id             uuid PRIMARY KEY,
    email          text NOT NULL CHECK (char_length(email) <= 320),
    password_hash  text NOT NULL,
    full_name      text NOT NULL CHECK (char_length(full_name) BETWEEN 1 AND 200),
    is_active      boolean NOT NULL DEFAULT true,
    last_login_at  timestamptz,
    created_at     timestamptz NOT NULL DEFAULT now(),
    updated_at     timestamptz NOT NULL DEFAULT now()
);

-- Case-insensitive uniqueness without requiring the citext extension.
CREATE UNIQUE INDEX users_email_lower_key ON users (lower(email));

CREATE TABLE organisation_members (
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    user_id          uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role             text NOT NULL
                     CHECK (role IN ('owner', 'admin', 'manager', 'staff', 'viewer')),
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, user_id)
);

CREATE INDEX organisation_members_user_idx ON organisation_members (user_id);

CREATE TABLE auth_sessions (
    id                  uuid PRIMARY KEY,
    user_id             uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    organisation_id     uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    refresh_token_hash  text NOT NULL,
    user_agent          text,
    expires_at          timestamptz NOT NULL,
    revoked_at          timestamptz,
    created_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT auth_sessions_refresh_token_hash_key UNIQUE (refresh_token_hash)
);

CREATE INDEX auth_sessions_user_idx ON auth_sessions (user_id);
CREATE INDEX auth_sessions_active_expiry_idx
    ON auth_sessions (expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE audit_log (
    id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    organisation_id  uuid REFERENCES organisations (id) ON DELETE SET NULL,
    actor_user_id    uuid REFERENCES users (id) ON DELETE SET NULL,
    action           text NOT NULL,
    entity_type      text NOT NULL,
    entity_id        uuid,
    metadata         jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX audit_log_organisation_created_idx
    ON audit_log (organisation_id, created_at DESC);

-- Keep `updated_at` honest without relying on application code.
CREATE OR REPLACE FUNCTION set_updated_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = now();
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_set_updated_at
    BEFORE UPDATE ON organisations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER users_set_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER organisation_members_set_updated_at
    BEFORE UPDATE ON organisation_members
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
