-- Contacts, external organisations and relationships.
--
-- Contacts are deliberately untyped: "client", "worker", "doctor" and "family"
-- are roles expressed through relationships and later assignments, not record
-- types. The intake process therefore uses one shape for everyone.

CREATE TABLE contacts (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    display_name     text NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 160),
    given_name       text,
    family_name      text,
    preferred_name   text,
    pronouns         text,
    date_of_birth    date,
    email            text,
    phone            text,
    notes            text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT contacts_id_org_key UNIQUE (organisation_id, id)
);

CREATE INDEX contacts_org_name_idx ON contacts (organisation_id, display_name);

COMMENT ON TABLE contacts IS
    'People known to the organisation. No type column by design: client, worker, doctor, guardian and family are relationship roles, not record types.';

-- External counterparties: providers, brokers, health services, agencies.
-- Distinct from `organisations`, which are CareLibre tenants.
CREATE TABLE external_organisations (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    kind             text,
    abn              text CHECK (abn IS NULL OR abn ~ '^[0-9]{11}$'),
    email            text,
    phone            text,
    website          text,
    notes            text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT external_organisations_id_org_key UNIQUE (organisation_id, id)
);

CREATE UNIQUE INDEX external_organisations_name_key
    ON external_organisations (organisation_id, lower(name));
CREATE INDEX external_organisations_kind_idx
    ON external_organisations (organisation_id, kind);

COMMENT ON TABLE external_organisations IS
    'Counterparties such as external providers, health services and brokering agencies. Not to be confused with `organisations` (CareLibre tenants).';
COMMENT ON COLUMN external_organisations.kind IS
    'Free-text descriptor, e.g. provider, broker, health, agency. Intentionally not an enum.';
COMMENT ON COLUMN external_organisations.abn IS
    'Australian Business Number, 11 digits.';

-- The glue: directed, typed links between contacts and external organisations.
-- At least one endpoint is a contact or an organisation; exactly one entity per
-- side. The relationship type is free text so new roles need no migration.
CREATE TABLE relationships (
    id                               uuid PRIMARY KEY,
    organisation_id                  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    from_contact_id                  uuid,
    from_external_organisation_id    uuid,
    to_contact_id                    uuid,
    to_external_organisation_id      uuid,
    relationship_type                text NOT NULL CHECK (char_length(relationship_type) BETWEEN 1 AND 64),
    inverse_name                     text,
    start_date                       date,
    end_date                         date,
    notes                            text,
    created_at                       timestamptz NOT NULL DEFAULT now(),
    updated_at                       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT relationships_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT relationships_from_exactly_one
        CHECK (num_nonnulls(from_contact_id, from_external_organisation_id) = 1),
    CONSTRAINT relationships_to_exactly_one
        CHECK (num_nonnulls(to_contact_id, to_external_organisation_id) = 1),
    CONSTRAINT relationships_distinct_endpoints CHECK (
        (from_contact_id IS NULL OR to_contact_id IS NULL OR from_contact_id <> to_contact_id)
        AND (from_external_organisation_id IS NULL OR to_external_organisation_id IS NULL
             OR from_external_organisation_id <> to_external_organisation_id)
    ),
    CONSTRAINT relationships_dates CHECK (
        end_date IS NULL OR start_date IS NULL OR end_date >= start_date
    ),
    CONSTRAINT relationships_from_contact_fk
        FOREIGN KEY (organisation_id, from_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT relationships_from_organisation_fk
        FOREIGN KEY (organisation_id, from_external_organisation_id)
        REFERENCES external_organisations (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT relationships_to_contact_fk
        FOREIGN KEY (organisation_id, to_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT relationships_to_organisation_fk
        FOREIGN KEY (organisation_id, to_external_organisation_id)
        REFERENCES external_organisations (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX relationships_from_contact_idx
    ON relationships (organisation_id, from_contact_id)
    WHERE from_contact_id IS NOT NULL;
CREATE INDEX relationships_to_contact_idx
    ON relationships (organisation_id, to_contact_id)
    WHERE to_contact_id IS NOT NULL;
CREATE INDEX relationships_from_org_idx
    ON relationships (organisation_id, from_external_organisation_id)
    WHERE from_external_organisation_id IS NOT NULL;
CREATE INDEX relationships_to_org_idx
    ON relationships (organisation_id, to_external_organisation_id)
    WHERE to_external_organisation_id IS NOT NULL;

COMMENT ON TABLE relationships IS
    'Typed directed links between contacts and/or external organisations. Roles such as guardian, doctor, case manager or broker live here rather than on the contact record.';

CREATE TRIGGER contacts_set_updated_at
    BEFORE UPDATE ON contacts
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER external_organisations_set_updated_at
    BEFORE UPDATE ON external_organisations
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER relationships_set_updated_at
    BEFORE UPDATE ON relationships
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
