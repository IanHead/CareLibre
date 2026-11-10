-- Funding management: sources, client packages, quotes/service agreements and
-- roster generation.
--
-- Funding sources are the programmes an organisation bills against: NDIS,
-- Support at Home, CHSP, private fee-for-service and arbitrary sources such as
-- TAC. Each source carries a claiming model so later claims work can branch on
-- it without changing the core tables.
--
-- A client holds one or more funding packages from those sources. A package is
-- quoted through an agreement; finalising the agreement turns its scheduled
-- lines into draft shifts, linked back to the package and line, which the
-- coordinator then fills with workers.
--
-- Claims submission, remittance and price-guide imports are deliberately a
-- later slice. This migration establishes the data those features need.

-- ---------------------------------------------------------------------------
-- 1. Funding sources
-- ---------------------------------------------------------------------------

CREATE TABLE funding_sources (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    key              text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{1,62}$'),
    name             text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    kind             text NOT NULL CHECK (kind IN (
        'ndis', 'support_at_home', 'chsp', 'private', 'other'
    )),
    claiming_model   text NOT NULL DEFAULT 'manual' CHECK (claiming_model IN (
        'ndis_price_guide', 'support_at_home', 'chsp', 'fee_for_service', 'manual'
    )),
    description      text,
    configuration    jsonb NOT NULL DEFAULT '{}'::jsonb,
    is_active        boolean NOT NULL DEFAULT true,
    is_system        boolean NOT NULL DEFAULT false,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT funding_sources_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT funding_sources_key_key UNIQUE (organisation_id, key),
    CONSTRAINT funding_sources_configuration_object CHECK (
        jsonb_typeof(configuration) = 'object'
    )
);

CREATE INDEX funding_sources_org_active_idx
    ON funding_sources (organisation_id, is_active, name);

CREATE TRIGGER funding_sources_set_updated_at
    BEFORE UPDATE ON funding_sources
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE funding_sources IS
    'Programmes the organisation bills against. The claiming_model selects the later claims workflow; configuration holds source-specific settings.';
COMMENT ON COLUMN funding_sources.claiming_model IS
    'ndis_price_guide, support_at_home, chsp, fee_for_service or manual. Claims code branches on this; no claim rows are created here.';

-- ---------------------------------------------------------------------------
-- 2. Client funding packages
-- ---------------------------------------------------------------------------

CREATE TABLE funding_packages (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    client_contact_id uuid NOT NULL,
    funding_source_id uuid NOT NULL,
    name              text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    reference         text,
    start_date        date NOT NULL,
    end_date          date,
    budget_amount     numeric(12,2) CHECK (budget_amount IS NULL OR budget_amount >= 0),
    status            text NOT NULL DEFAULT 'active' CHECK (status IN (
        'draft', 'active', 'exhausted', 'closed'
    )),
    notes             text,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT funding_packages_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT funding_packages_client_fk
        FOREIGN KEY (organisation_id, client_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT funding_packages_source_fk
        FOREIGN KEY (organisation_id, funding_source_id)
        REFERENCES funding_sources (organisation_id, id) ON DELETE RESTRICT,
    CONSTRAINT funding_packages_dates CHECK (
        end_date IS NULL OR end_date >= start_date
    )
);

CREATE INDEX funding_packages_client_idx
    ON funding_packages (organisation_id, client_contact_id, start_date DESC);
CREATE INDEX funding_packages_source_idx
    ON funding_packages (organisation_id, funding_source_id);

CREATE TRIGGER funding_packages_set_updated_at
    BEFORE UPDATE ON funding_packages
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE funding_packages IS
    'A funding allocation a client holds from a source, e.g. an NDIS plan, a Support at Home package or a private quote.';

-- ---------------------------------------------------------------------------
-- 3. Quotes and service agreements
-- ---------------------------------------------------------------------------

CREATE TABLE client_agreements (
    id                 uuid PRIMARY KEY,
    organisation_id    uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    client_contact_id  uuid NOT NULL,
    funding_package_id uuid NOT NULL,
    title              text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    status             text NOT NULL DEFAULT 'draft' CHECK (status IN (
        'draft', 'quoted', 'accepted', 'active', 'completed', 'cancelled'
    )),
    quote_date         date,
    start_date         date NOT NULL,
    end_date           date,
    total_amount       numeric(12,2) NOT NULL DEFAULT 0 CHECK (total_amount >= 0),
    notes              text,
    finalised_at       timestamptz,
    created_at         timestamptz NOT NULL DEFAULT now(),
    updated_at         timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT client_agreements_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT client_agreements_client_fk
        FOREIGN KEY (organisation_id, client_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_agreements_package_fk
        FOREIGN KEY (organisation_id, funding_package_id)
        REFERENCES funding_packages (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT client_agreements_dates CHECK (
        end_date IS NULL OR end_date >= start_date
    )
);

CREATE INDEX client_agreements_client_idx
    ON client_agreements (organisation_id, client_contact_id, start_date DESC);
CREATE INDEX client_agreements_package_idx
    ON client_agreements (organisation_id, funding_package_id);

CREATE TRIGGER client_agreements_set_updated_at
    BEFORE UPDATE ON client_agreements
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE client_agreements IS
    'A quote and, once accepted and finalised, the service agreement for a funding package. Finalising generates draft shifts.';

CREATE TABLE agreement_lines (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    agreement_id     uuid NOT NULL,
    service_id       uuid,
    description      text NOT NULL CHECK (char_length(description) BETWEEN 1 AND 300),
    quantity         numeric(10,2) NOT NULL DEFAULT 1 CHECK (quantity > 0),
    unit             text NOT NULL DEFAULT 'hour' CHECK (unit IN (
        'hour', 'session', 'each', 'day'
    )),
    unit_rate        numeric(12,2) NOT NULL CHECK (unit_rate >= 0),
    amount           numeric(12,2) NOT NULL CHECK (amount >= 0),
    recurrence       text NOT NULL DEFAULT 'none' CHECK (recurrence IN (
        'none', 'daily', 'weekly', 'fortnightly'
    )),
    weekdays         smallint[] NOT NULL DEFAULT '{}',
    start_time       time,
    duration_minutes integer CHECK (duration_minutes IS NULL OR duration_minutes > 0),
    site_id          uuid,
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT agreement_lines_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT agreement_lines_agreement_fk
        FOREIGN KEY (organisation_id, agreement_id)
        REFERENCES client_agreements (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT agreement_lines_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id) ON DELETE SET NULL (service_id),
    CONSTRAINT agreement_lines_site_fk
        FOREIGN KEY (organisation_id, site_id)
        REFERENCES sites (organisation_id, id) ON DELETE SET NULL (site_id),
    CONSTRAINT agreement_lines_weekdays CHECK (
        weekdays <@ ARRAY[1,2,3,4,5,6,7]::smallint[]
    )
);

CREATE INDEX agreement_lines_agreement_idx
    ON agreement_lines (organisation_id, agreement_id, id);

CREATE TRIGGER agreement_lines_set_updated_at
    BEFORE UPDATE ON agreement_lines
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

COMMENT ON TABLE agreement_lines IS
    'One quoted service line. A line with a recurrence and weekdays becomes one draft shift per occurrence when the agreement is finalised.';

-- ---------------------------------------------------------------------------
-- 4. Link generated shifts back to funding
-- ---------------------------------------------------------------------------

ALTER TABLE shifts
    ADD COLUMN funding_package_id uuid,
    ADD COLUMN agreement_line_id uuid,
    ADD CONSTRAINT shifts_funding_package_fk
        FOREIGN KEY (organisation_id, funding_package_id)
        REFERENCES funding_packages (organisation_id, id)
        ON DELETE SET NULL (funding_package_id),
    ADD CONSTRAINT shifts_agreement_line_fk
        FOREIGN KEY (organisation_id, agreement_line_id)
        REFERENCES agreement_lines (organisation_id, id)
        ON DELETE SET NULL (agreement_line_id);

CREATE INDEX shifts_funding_package_idx
    ON shifts (organisation_id, funding_package_id)
    WHERE funding_package_id IS NOT NULL;
CREATE INDEX shifts_agreement_line_idx
    ON shifts (organisation_id, agreement_line_id)
    WHERE agreement_line_id IS NOT NULL;

COMMENT ON COLUMN shifts.funding_package_id IS
    'Funding package the generated shift is billed against. Set when an agreement is finalised.';
COMMENT ON COLUMN shifts.agreement_line_id IS
    'Agreement line that generated this shift, so quoting and claims can trace back.';

-- ---------------------------------------------------------------------------
-- 5. Package summary view
-- ---------------------------------------------------------------------------

CREATE VIEW funding_package_summary AS
SELECT
    p.organisation_id,
    p.id AS funding_package_id,
    p.client_contact_id,
    p.name,
    p.reference,
    p.status,
    p.start_date,
    p.end_date,
    p.budget_amount,
    COALESCE(SUM(a.total_amount) FILTER (
        WHERE a.status IN ('active', 'completed')
    ), 0)::numeric(12,2) AS committed_amount,
    CASE
        WHEN p.budget_amount IS NULL THEN NULL
        ELSE (p.budget_amount - COALESCE(SUM(a.total_amount) FILTER (
            WHERE a.status IN ('active', 'completed')
        ), 0))::numeric(12,2)
    END AS remaining_amount
FROM funding_packages p
LEFT JOIN client_agreements a
  ON a.organisation_id = p.organisation_id
 AND a.funding_package_id = p.id
GROUP BY p.organisation_id, p.id;

COMMENT ON VIEW funding_package_summary IS
    'Funding packages with the total value of active/completed agreements and the remaining budget.';

-- ---------------------------------------------------------------------------
-- 6. Seed the standard sources
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION carelibre_seed_funding_sources(p_organisation_id uuid)
RETURNS void
LANGUAGE sql AS $$
    INSERT INTO funding_sources
        (id, organisation_id, key, name, kind, claiming_model, description,
         is_active, is_system)
    SELECT gen_random_uuid(), p_organisation_id, seed.key, seed.name, seed.kind,
           seed.claiming_model, seed.description, true, true
    FROM (
        VALUES
            ('ndis', 'NDIS', 'ndis', 'ndis_price_guide',
             'National Disability Insurance Scheme plans and supports.'),
            ('support_at_home', 'Support at Home', 'support_at_home', 'support_at_home',
             'Aged care Support at Home packages.'),
            ('chsp', 'CHSP', 'chsp', 'chsp',
             'Commonwealth Home Support Programme.'),
            ('private', 'Private / fee for service', 'private', 'fee_for_service',
             'Privately funded services billed directly to the client.'),
            ('tac', 'TAC funding', 'other', 'manual',
             'Transport Accident Commission or another arbitrary funding source.')
    ) AS seed (key, name, kind, claiming_model, description)
    ON CONFLICT DO NOTHING;
$$;

CREATE OR REPLACE FUNCTION seed_funding_sources_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_funding_sources(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_seed_funding_sources
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION seed_funding_sources_for_organisation();

SELECT carelibre_seed_funding_sources(id) FROM organisations WHERE tenant_id = id;
