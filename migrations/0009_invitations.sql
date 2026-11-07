-- One-time invitation links.
--
-- The plaintext token is never stored; only its SHA-256 hash. Links are
-- single-use, revocable and expire. Permission sets travel with the invitation
-- and are applied at acceptance.

CREATE TABLE invitations (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    email             text NOT NULL CHECK (char_length(email) BETWEEN 3 AND 320),
    full_name         text,
    token_hash        text NOT NULL,
    expires_at        timestamptz NOT NULL,
    accepted_at       timestamptz,
    accepted_user_id  uuid REFERENCES users (id) ON DELETE SET NULL,
    revoked_at        timestamptz,
    created_by        uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT invitations_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT invitations_token_key UNIQUE (token_hash),
    CONSTRAINT invitations_expiry CHECK (expires_at > created_at)
);

-- One pending invitation per email per organisation.
CREATE UNIQUE INDEX invitations_pending_key
    ON invitations (organisation_id, lower(email))
    WHERE accepted_at IS NULL AND revoked_at IS NULL;

CREATE INDEX invitations_org_created_idx
    ON invitations (organisation_id, created_at DESC);

COMMENT ON TABLE invitations IS
    'One-time invitation links. The plaintext token is shown once and stored only as a SHA-256 hash.';

CREATE TABLE invitation_permission_sets (
    organisation_id   uuid NOT NULL,
    invitation_id     uuid NOT NULL,
    permission_set_id uuid NOT NULL,
    PRIMARY KEY (invitation_id, permission_set_id),
    CONSTRAINT invitation_permission_sets_invitation_fk
        FOREIGN KEY (organisation_id, invitation_id)
        REFERENCES invitations (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT invitation_permission_sets_set_fk
        FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE RESTRICT
);

CREATE INDEX invitation_permission_sets_set_idx
    ON invitation_permission_sets (permission_set_id);

COMMENT ON TABLE invitation_permission_sets IS
    'Permission sets granted when the invitation is accepted.';
