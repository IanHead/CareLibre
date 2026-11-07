-- Divisions, services, rates and effective-dated rate versions.

CREATE TABLE divisions (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    parent_id        uuid,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    code             text CHECK (code IS NULL OR char_length(code) BETWEEN 1 AND 32),
    description      text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT divisions_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT divisions_parent_fk
        FOREIGN KEY (organisation_id, parent_id)
        REFERENCES divisions (organisation_id, id),
    CONSTRAINT divisions_not_own_parent CHECK (parent_id IS NULL OR parent_id <> id)
);

CREATE UNIQUE INDEX divisions_code_key
    ON divisions (organisation_id, code)
    WHERE code IS NOT NULL;
CREATE INDEX divisions_parent_idx ON divisions (organisation_id, parent_id);

COMMENT ON TABLE divisions IS
    'Organisational units. Hierarchical; a division may own services and carry a cost centre.';

CREATE TABLE services (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    division_id      uuid,
    code             text NOT NULL CHECK (char_length(code) BETWEEN 1 AND 64),
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 200),
    description      text,
    funding_scheme   text NOT NULL CHECK (funding_scheme IN ('ndis', 'aged_care', 'other')),
    funding_code     text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT services_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT services_code_key UNIQUE (organisation_id, code),
    CONSTRAINT services_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id)
);

CREATE INDEX services_funding_idx
    ON services (organisation_id, funding_scheme, funding_code);

COMMENT ON TABLE services IS
    'Billable services mapped to NDIS or aged-care funding codes. One service holds many rates.';
COMMENT ON COLUMN services.funding_scheme IS
    'ndis = NDIS support item, aged_care = CHSP/SAH or other aged-care program, other = non-funded.';
COMMENT ON COLUMN services.funding_code IS
    'NDIS support item number or aged-care code, as published by the funding body.';

CREATE TABLE rates (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    service_id       uuid NOT NULL,
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    unit_of_measure  text NOT NULL DEFAULT 'hour' CHECK (char_length(unit_of_measure) BETWEEN 1 AND 32),
    description      text,
    is_active        boolean NOT NULL DEFAULT true,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT rates_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT rates_name_key UNIQUE (organisation_id, service_id, name),
    CONSTRAINT rates_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX rates_service_idx ON rates (organisation_id, service_id);

COMMENT ON TABLE rates IS
    'Named price for a service, with a unit of measure (hour, day, each, kilometre...). Prices live in rate_versions.';
COMMENT ON COLUMN rates.unit_of_measure IS
    'Billing unit, e.g. hour, day, session, each, kilometre. Free text so new units need no migration.';

CREATE TABLE rate_versions (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    rate_id          uuid NOT NULL,
    amount           numeric(12, 2) NOT NULL CHECK (amount >= 0),
    currency         char(3) NOT NULL DEFAULT 'AUD',
    effective_from   date NOT NULL,
    effective_to     date,
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT rate_versions_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT rate_versions_dates CHECK (effective_to IS NULL OR effective_to > effective_from),
    CONSTRAINT rate_versions_rate_fk
        FOREIGN KEY (organisation_id, rate_id)
        REFERENCES rates (organisation_id, id) ON DELETE CASCADE
);

-- One version per start date, and at most one open-ended version per rate.
CREATE UNIQUE INDEX rate_versions_start_key
    ON rate_versions (organisation_id, rate_id, effective_from);
CREATE UNIQUE INDEX rate_versions_open_key
    ON rate_versions (organisation_id, rate_id)
    WHERE effective_to IS NULL;

COMMENT ON TABLE rate_versions IS
    'Effective-dated prices, e.g. "Effective 1 July 2027: $98.65". Ranges are half-open [effective_from, effective_to).';

-- Overlap protection without requiring the btree_gist extension (managed
-- PostgreSQL instances do not always allow extension creation).
CREATE OR REPLACE FUNCTION reject_rate_version_overlap() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM rate_versions rv
        WHERE rv.organisation_id = NEW.organisation_id
          AND rv.rate_id = NEW.rate_id
          AND rv.id <> NEW.id
          AND daterange(rv.effective_from, rv.effective_to, '[)')
              && daterange(NEW.effective_from, NEW.effective_to, '[)')
    ) THEN
        RAISE EXCEPTION 'rate version overlaps an existing version for rate %', NEW.rate_id
            USING ERRCODE = 'exclusion_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER rate_versions_no_overlap
    BEFORE INSERT OR UPDATE ON rate_versions
    FOR EACH ROW EXECUTE FUNCTION reject_rate_version_overlap();

CREATE TRIGGER divisions_set_updated_at
    BEFORE UPDATE ON divisions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER services_set_updated_at
    BEFORE UPDATE ON services
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER rates_set_updated_at
    BEFORE UPDATE ON rates
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER rate_versions_set_updated_at
    BEFORE UPDATE ON rate_versions
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
