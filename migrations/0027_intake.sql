-- Intake cases, referrals, waitlists and the contact lifecycle.
--
-- The kanban columns are intake stages, not contact statuses. A contact is
-- created at enquiry so repeated attempts reuse the same person; the
-- care/employment relationship is linked when the case onboards.
--
-- See docs/adr/0013-intake-cases.md and docs/intake.md.

-- ---------------------------------------------------------------------------
-- Contact lifecycle
-- ---------------------------------------------------------------------------

ALTER TABLE contacts
    ADD COLUMN lifecycle_status text NOT NULL DEFAULT 'active'
        CHECK (lifecycle_status IN (
            'enquiry', 'onboarding', 'suspended', 'active',
            'offboarding', 'intake_cancelled', 'offboarded'
        )),
    ADD COLUMN lifecycle_changed_at timestamptz NOT NULL DEFAULT now();

CREATE INDEX contacts_lifecycle_idx
    ON contacts (organisation_id, lifecycle_status);

COMMENT ON COLUMN contacts.lifecycle_status IS
    'Operational lifecycle, independent of intake stages. See docs/intake.md.';

CREATE TABLE contact_status_history (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    contact_id       uuid NOT NULL,
    from_status      text CHECK (from_status IS NULL OR from_status IN (
                         'enquiry', 'onboarding', 'suspended', 'active',
                         'offboarding', 'intake_cancelled', 'offboarded')),
    to_status        text NOT NULL CHECK (to_status IN (
                         'enquiry', 'onboarding', 'suspended', 'active',
                         'offboarding', 'intake_cancelled', 'offboarded')),
    reason           text,
    actor_user_id    uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT contact_status_history_contact_fk
        FOREIGN KEY (organisation_id, contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX contact_status_history_contact_idx
    ON contact_status_history (organisation_id, contact_id, created_at DESC);

CREATE OR REPLACE FUNCTION set_contact_lifecycle_changed_at() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.lifecycle_status IS DISTINCT FROM OLD.lifecycle_status THEN
        NEW.lifecycle_changed_at := now();
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER contacts_set_lifecycle_changed_at
    BEFORE UPDATE ON contacts
    FOR EACH ROW EXECUTE FUNCTION set_contact_lifecycle_changed_at();

COMMENT ON TABLE contact_status_history IS
    'Append-only history of contact lifecycle changes.';

-- Intake cases can carry custom-field values through the existing validation
-- trigger.
INSERT INTO custom_field_entities (entity_type, label, sort_order)
VALUES ('intake_case', 'Intake cases', 25)
ON CONFLICT (entity_type) DO NOTHING;

-- ---------------------------------------------------------------------------
-- Capabilities
-- ---------------------------------------------------------------------------

INSERT INTO capabilities (key, category, label, description, sort_order) VALUES
    ('intake.read',      'Intake', 'View intake',      'View intake flows, cases, waitlists and timelines', 16),
    ('intake.write',     'Intake', 'Edit intake',      'Create, edit, transition and archive intake cases', 17),
    ('intake.assign',    'Intake', 'Assign intake',    'Reassign intake cases and stage queues', 18),
    ('intake.configure', 'Intake', 'Configure intake', 'Manage intake flows, stages, fields, actions and checklists', 19)
ON CONFLICT (key) DO NOTHING;

-- Existing Administrator sets must gain the new capabilities. The capability
-- guard prevents editing immutable Administrator sets, so pause it for this
-- migration only.
ALTER TABLE permission_set_capabilities DISABLE TRIGGER permission_set_capabilities_guard;

INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, c.key
FROM permission_sets ps
CROSS JOIN capabilities c
WHERE ps.persona_key = 'administrator'
  AND c.key IN ('intake.read', 'intake.write', 'intake.assign', 'intake.configure')
ON CONFLICT DO NOTHING;

ALTER TABLE permission_set_capabilities ENABLE TRIGGER permission_set_capabilities_guard;

-- Grant intake capabilities to the editable OOTB personas. Custom sets are
-- deliberately untouched.
INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
SELECT ps.organisation_id, ps.id, v.capability
FROM permission_sets ps
JOIN (VALUES
    ('intake', 'intake.read'),
    ('intake', 'intake.write'),
    ('intake', 'intake.assign'),
    ('senior_management', 'intake.read'),
    ('house_manager', 'intake.read')
) AS v(persona_key, capability)
  ON v.persona_key = ps.persona_key
ON CONFLICT DO NOTHING;

CREATE OR REPLACE FUNCTION carelibre_seed_intake_permission_sets(p_organisation_id uuid)
RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    -- Administrator already receives every capability when the persona is
    -- seeded, and is immutable. Only editable personas need new grants.
    INSERT INTO permission_set_capabilities (organisation_id, permission_set_id, capability)
    SELECT ps.organisation_id, ps.id, v.capability
    FROM permission_sets ps
    JOIN (VALUES
        ('intake', 'intake.read'),
        ('intake', 'intake.write'),
        ('intake', 'intake.assign'),
        ('senior_management', 'intake.read'),
        ('house_manager', 'intake.read')
    ) AS v(persona_key, capability)
      ON v.persona_key = ps.persona_key
    WHERE ps.organisation_id = p_organisation_id
    ON CONFLICT DO NOTHING;
END;
$$;

-- Future organisations seed the base personas first, then the intake grants.
CREATE OR REPLACE FUNCTION seed_permission_sets_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM carelibre_seed_permission_sets(NEW.id);
    PERFORM carelibre_seed_intake_permission_sets(NEW.id);
    RETURN NEW;
END;
$$;

-- ---------------------------------------------------------------------------
-- Flows and stages
-- ---------------------------------------------------------------------------

CREATE TABLE intake_flows (
    id                uuid PRIMARY KEY,
    organisation_id   uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    parent_flow_id    uuid,
    key               text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{1,63}$'),
    name              text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    subject_kind      text NOT NULL CHECK (subject_kind IN (
                          'client', 'worker', 'other_contact', 'organisation')),
    variant           text CHECK (variant IS NULL OR char_length(variant) BETWEEN 1 AND 64),
    description       text,
    division_id       uuid,
    is_default        boolean NOT NULL DEFAULT false,
    is_active         boolean NOT NULL DEFAULT true,
    sort_order        integer NOT NULL DEFAULT 0,
    created_at        timestamptz NOT NULL DEFAULT now(),
    updated_at        timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT intake_flows_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT intake_flows_key_key UNIQUE (organisation_id, key),
    CONSTRAINT intake_flows_parent_fk
        FOREIGN KEY (organisation_id, parent_flow_id)
        REFERENCES intake_flows (organisation_id, id),
    CONSTRAINT intake_flows_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id)
);

CREATE UNIQUE INDEX intake_flows_one_default
    ON intake_flows (organisation_id, subject_kind, COALESCE(variant, ''))
    WHERE is_default AND is_active;

CREATE INDEX intake_flows_org_sort_idx
    ON intake_flows (organisation_id, subject_kind, sort_order);

COMMENT ON TABLE intake_flows IS
    'Configurable intake pipelines. Stages are the kanban columns.';

CREATE TABLE intake_stages (
    id                      uuid PRIMARY KEY,
    organisation_id         uuid NOT NULL,
    flow_id                 uuid NOT NULL,
    key                     text NOT NULL CHECK (key ~ '^[a-z][a-z0-9_]{1,63}$'),
    name                    text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 160),
    stage_kind              text NOT NULL CHECK (stage_kind IN (
                                'enquiry', 'eligibility', 'risk_assessment',
                                'service_planning', 'custom', 'onboarded', 'cancelled')),
    color                   text CHECK (color IS NULL OR char_length(color) BETWEEN 1 AND 32),
    sort_order              integer NOT NULL DEFAULT 0,
    is_terminal             boolean NOT NULL DEFAULT false,
    on_enter_contact_status text CHECK (on_enter_contact_status IS NULL OR
                                on_enter_contact_status IN (
                                    'enquiry', 'onboarding', 'active',
                                    'intake_cancelled')),
    sla_hours               integer CHECK (sla_hours IS NULL OR sla_hours > 0),
    checklist_gate          boolean NOT NULL DEFAULT false,
    visible_fields          jsonb NOT NULL DEFAULT '[]'::jsonb,
    required_fields         jsonb NOT NULL DEFAULT '[]'::jsonb,
    created_at              timestamptz NOT NULL DEFAULT now(),
    updated_at              timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT intake_stages_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT intake_stages_flow_key_key UNIQUE (organisation_id, flow_id, key),
    CONSTRAINT intake_stages_terminal_kind
        CHECK (is_terminal = (stage_kind IN ('onboarded', 'cancelled'))),
    CONSTRAINT intake_stages_visible_array
        CHECK (jsonb_typeof(visible_fields) = 'array'),
    CONSTRAINT intake_stages_required_array
        CHECK (jsonb_typeof(required_fields) = 'array'),
    CONSTRAINT intake_stages_flow_fk
        FOREIGN KEY (organisation_id, flow_id)
        REFERENCES intake_flows (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX intake_stages_flow_order_idx
    ON intake_stages (organisation_id, flow_id, sort_order);

COMMENT ON TABLE intake_stages IS
    'Ordered columns in a flow. stage_kind carries code semantics; label and colour are configurable.';

CREATE TABLE intake_stage_actions (
    organisation_id  uuid NOT NULL,
    stage_id         uuid NOT NULL,
    action_key       text NOT NULL CHECK (action_key IN (
                         'roster.add', 'roster.remove',
                         'funding.add', 'funding.edit', 'funding.remove',
                         'profile.edit', 'relationships.update')),
    allowed          boolean NOT NULL DEFAULT false,
    PRIMARY KEY (organisation_id, stage_id, action_key),
    CONSTRAINT intake_stage_actions_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id) ON DELETE CASCADE
);

CREATE TABLE intake_stage_assignees (
    id                 uuid PRIMARY KEY,
    organisation_id    uuid NOT NULL,
    stage_id           uuid NOT NULL,
    user_id            uuid REFERENCES users (id) ON DELETE CASCADE,
    permission_set_id  uuid,
    strategy           text NOT NULL DEFAULT 'fixed'
                       CHECK (strategy IN ('fixed', 'round_robin', 'first_available')),
    notify             boolean NOT NULL DEFAULT true,
    sort_order         integer NOT NULL DEFAULT 0,
    CONSTRAINT intake_stage_assignees_target
        CHECK (num_nonnulls(user_id, permission_set_id) = 1),
    CONSTRAINT intake_stage_assignees_fixed_user
        CHECK (strategy <> 'fixed' OR user_id IS NOT NULL),
    CONSTRAINT intake_stage_assignees_pool_strategy
        CHECK (strategy = 'fixed' OR permission_set_id IS NOT NULL),
    CONSTRAINT intake_stage_assignees_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT intake_stage_assignees_permission_set_fk
        FOREIGN KEY (organisation_id, permission_set_id)
        REFERENCES permission_sets (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX intake_stage_assignees_stage_idx
    ON intake_stage_assignees (organisation_id, stage_id, sort_order);

CREATE TABLE intake_stage_checklist_items (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL,
    stage_id         uuid NOT NULL,
    label            text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 200),
    description      text,
    is_required      boolean NOT NULL DEFAULT true,
    sort_order       integer NOT NULL DEFAULT 0,
    CONSTRAINT intake_stage_checklist_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX intake_stage_checklist_stage_idx
    ON intake_stage_checklist_items (organisation_id, stage_id, sort_order);

CREATE TABLE intake_assignment_cursors (
    organisation_id  uuid NOT NULL,
    stage_id         uuid NOT NULL,
    last_user_id     uuid REFERENCES users (id) ON DELETE SET NULL,
    updated_at       timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (organisation_id, stage_id),
    CONSTRAINT intake_assignment_cursors_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id) ON DELETE CASCADE
);

-- ---------------------------------------------------------------------------
-- Cases
-- ---------------------------------------------------------------------------

CREATE TABLE intake_cases (
    id                        uuid PRIMARY KEY,
    organisation_id           uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    flow_id                   uuid NOT NULL,
    stage_id                  uuid NOT NULL,
    contact_id                uuid,
    external_organisation_id  uuid,
    subject_kind              text NOT NULL CHECK (subject_kind IN (
                                  'client', 'worker', 'other_contact', 'organisation')),
    display_name              text NOT NULL CHECK (char_length(display_name) BETWEEN 1 AND 200),
    data                      jsonb NOT NULL DEFAULT '{}'::jsonb,
    source                    text NOT NULL DEFAULT 'enquiry'
                              CHECK (source IN ('enquiry', 'referral', 'internal', 'waitlist')),
    referrer_contact_id       uuid,
    referrer_organisation_id  uuid,
    referral_reference        text,
    referral_received_at      timestamptz,
    division_id               uuid,
    assignee_user_id          uuid REFERENCES users (id) ON DELETE SET NULL,
    priority                  smallint NOT NULL DEFAULT 0,
    stage_entered_at          timestamptz NOT NULL DEFAULT now(),
    stage_due_at              timestamptz,
    version                   integer NOT NULL DEFAULT 1 CHECK (version > 0),
    created_by                uuid REFERENCES users (id) ON DELETE SET NULL,
    created_at                timestamptz NOT NULL DEFAULT now(),
    updated_at                timestamptz NOT NULL DEFAULT now(),
    onboarded_at              timestamptz,
    closed_at                 timestamptz,
    cancellation_reason       text,
    client_request_id         text,
    legacy_draft_id           text,
    CONSTRAINT intake_cases_id_org_key UNIQUE (organisation_id, id),
    CONSTRAINT intake_cases_flow_fk
        FOREIGN KEY (organisation_id, flow_id)
        REFERENCES intake_flows (organisation_id, id),
    CONSTRAINT intake_cases_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id),
    CONSTRAINT intake_cases_contact_fk
        FOREIGN KEY (organisation_id, contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE RESTRICT,
    CONSTRAINT intake_cases_external_org_fk
        FOREIGN KEY (organisation_id, external_organisation_id)
        REFERENCES organisations (tenant_id, id) ON DELETE RESTRICT,
    CONSTRAINT intake_cases_referrer_contact_fk
        FOREIGN KEY (organisation_id, referrer_contact_id)
        REFERENCES contacts (organisation_id, id) ON DELETE SET NULL,
    CONSTRAINT intake_cases_referrer_org_fk
        FOREIGN KEY (organisation_id, referrer_organisation_id)
        REFERENCES organisations (tenant_id, id) ON DELETE SET NULL,
    CONSTRAINT intake_cases_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id),
    CONSTRAINT intake_cases_subject_link
        CHECK (
            (subject_kind = 'organisation'
                AND contact_id IS NULL AND external_organisation_id IS NOT NULL)
            OR (subject_kind <> 'organisation'
                AND contact_id IS NOT NULL AND external_organisation_id IS NULL)
        ),
    CONSTRAINT intake_cases_referrers
        CHECK (num_nonnulls(referrer_contact_id, referrer_organisation_id) <= 1),
    CONSTRAINT intake_cases_data_object
        CHECK (jsonb_typeof(data) = 'object')
);

CREATE INDEX intake_cases_board_idx
    ON intake_cases (organisation_id, flow_id, stage_id, priority DESC, stage_entered_at, id);

CREATE INDEX intake_cases_assignee_idx
    ON intake_cases (organisation_id, assignee_user_id, updated_at DESC)
    WHERE closed_at IS NULL;

CREATE INDEX intake_cases_division_idx
    ON intake_cases (organisation_id, division_id)
    WHERE closed_at IS NULL;

CREATE INDEX intake_cases_org_name_idx
    ON intake_cases (organisation_id, lower(display_name));

CREATE UNIQUE INDEX intake_cases_one_open_per_flow
    ON intake_cases (organisation_id, contact_id, flow_id)
    WHERE contact_id IS NOT NULL AND closed_at IS NULL AND onboarded_at IS NULL;

CREATE UNIQUE INDEX intake_cases_client_request_key
    ON intake_cases (organisation_id, client_request_id)
    WHERE client_request_id IS NOT NULL;

CREATE UNIQUE INDEX intake_cases_legacy_draft_key
    ON intake_cases (organisation_id, legacy_draft_id)
    WHERE legacy_draft_id IS NOT NULL;

COMMENT ON TABLE intake_cases IS
    'One attempt at intake. A contact may have several cases; at most one open per flow.';

CREATE TABLE intake_case_checklist_items (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL,
    case_id          uuid NOT NULL,
    stage_id         uuid NOT NULL,
    template_item_id uuid,
    label            text NOT NULL CHECK (char_length(label) BETWEEN 1 AND 200),
    is_required      boolean NOT NULL DEFAULT true,
    completed_at     timestamptz,
    completed_by     uuid REFERENCES users (id) ON DELETE SET NULL,
    note             text,
    sort_order       integer NOT NULL DEFAULT 0,
    CONSTRAINT intake_case_checklist_case_fk
        FOREIGN KEY (organisation_id, case_id)
        REFERENCES intake_cases (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT intake_case_checklist_stage_fk
        FOREIGN KEY (organisation_id, stage_id)
        REFERENCES intake_stages (organisation_id, id) ON DELETE RESTRICT
);

CREATE INDEX intake_case_checklist_case_idx
    ON intake_case_checklist_items (organisation_id, case_id, stage_id, sort_order);

CREATE TABLE intake_case_events (
    id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    organisation_id  uuid NOT NULL,
    case_id          uuid NOT NULL,
    event_type       text NOT NULL CHECK (event_type IN (
                         'created', 'updated', 'transitioned', 'assigned',
                         'checklist_completed', 'checklist_reopened', 'waitlisted',
                         'cancelled', 'onboarded', 'archived', 'reopened',
                         'action_denied')),
    actor_user_id    uuid REFERENCES users (id) ON DELETE SET NULL,
    from_stage_id    uuid,
    to_stage_id      uuid,
    metadata         jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT intake_case_events_case_fk
        FOREIGN KEY (organisation_id, case_id)
        REFERENCES intake_cases (organisation_id, id) ON DELETE CASCADE
);

CREATE INDEX intake_case_events_case_idx
    ON intake_case_events (organisation_id, case_id, created_at DESC, id DESC);

CREATE TABLE intake_waitlist_entries (
    id               uuid PRIMARY KEY,
    organisation_id  uuid NOT NULL,
    case_id          uuid NOT NULL,
    service_id       uuid,
    division_id      uuid,
    priority         smallint NOT NULL DEFAULT 0,
    requested_start  date,
    status           text NOT NULL DEFAULT 'waiting'
                     CHECK (status IN ('waiting', 'offered', 'placed', 'withdrawn')),
    offered_at       timestamptz,
    offer_expires_at timestamptz,
    notes            text,
    created_at       timestamptz NOT NULL DEFAULT now(),
    updated_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT intake_waitlist_case_fk
        FOREIGN KEY (organisation_id, case_id)
        REFERENCES intake_cases (organisation_id, id) ON DELETE CASCADE,
    CONSTRAINT intake_waitlist_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id) ON DELETE SET NULL,
    CONSTRAINT intake_waitlist_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id) ON DELETE SET NULL
);

CREATE INDEX intake_waitlist_board_idx
    ON intake_waitlist_entries (organisation_id, status, division_id, priority DESC, created_at);

CREATE TABLE notifications (
    id               bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    organisation_id  uuid NOT NULL REFERENCES organisations (id) ON DELETE CASCADE,
    user_id          uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind             text NOT NULL,
    title            text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    body             text,
    entity_type      text,
    entity_id        uuid,
    dedupe_key       text,
    read_at          timestamptz,
    created_at       timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX notifications_dedupe_key
    ON notifications (organisation_id, user_id, dedupe_key)
    WHERE dedupe_key IS NOT NULL;

CREATE INDEX notifications_unread_idx
    ON notifications (organisation_id, user_id, created_at DESC)
    WHERE read_at IS NULL;

COMMENT ON TABLE notifications IS
    'In-app notifications. Email/SMS delivery can later consume the same rows.';

CREATE TRIGGER intake_flows_set_updated_at
    BEFORE UPDATE ON intake_flows
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER intake_stages_set_updated_at
    BEFORE UPDATE ON intake_stages
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER intake_cases_set_updated_at
    BEFORE UPDATE ON intake_cases
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TRIGGER intake_waitlist_set_updated_at
    BEFORE UPDATE ON intake_waitlist_entries
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ---------------------------------------------------------------------------
-- Default flows and stages
-- ---------------------------------------------------------------------------

CREATE OR REPLACE FUNCTION carelibre_seed_intake_flows(p_organisation_id uuid)
RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_flow record;
BEGIN
    -- Only tenant roots own intake flows; external organisations and branches
    -- share their tenant's configuration.
    IF NOT EXISTS (
        SELECT 1 FROM organisations
        WHERE id = p_organisation_id AND tenant_id = id
    ) THEN
        RETURN;
    END IF;

    IF EXISTS (
        SELECT 1 FROM intake_flows WHERE organisation_id = p_organisation_id
    ) THEN
        RETURN;
    END IF;

    INSERT INTO intake_flows
        (id, organisation_id, key, name, subject_kind, variant, description, sort_order)
    VALUES
        (gen_random_uuid(), p_organisation_id, 'client_standard',
         'Client — standard', 'client', 'standard',
         'Default client intake', 10),
        (gen_random_uuid(), p_organisation_id, 'client_aged_care',
         'Client — aged care', 'client', 'aged_care',
         'Aged-care intake', 20),
        (gen_random_uuid(), p_organisation_id, 'client_disability',
         'Client — disability', 'client', 'disability',
         'Parent disability flow and generic disability intake', 25),
        (gen_random_uuid(), p_organisation_id, 'client_disability_standard',
         'Client — disability standard', 'client', 'standard',
         'Standard disability intake', 30),
        (gen_random_uuid(), p_organisation_id, 'client_disability_emergency',
         'Client — disability emergency', 'client', 'emergency',
         'Emergency disability intake', 35),
        (gen_random_uuid(), p_organisation_id, 'client_fee_for_service',
         'Client — fee for service', 'client', 'fee_for_service',
         'Private and fee-for-service intake', 40),
        (gen_random_uuid(), p_organisation_id, 'worker_standard',
         'Worker — standard', 'worker', 'standard',
         'Worker onboarding', 50),
        (gen_random_uuid(), p_organisation_id, 'other_contact_standard',
         'Other contact — standard', 'other_contact', 'standard',
         'Coordinators, family, GPs and other network contacts', 60),
        (gen_random_uuid(), p_organisation_id, 'organisation_standard',
         'Organisation — standard', 'organisation', 'standard',
         'External provider, broker or agency intake', 70);

    UPDATE intake_flows AS child
    SET parent_flow_id = parent.id
    FROM intake_flows AS parent
    WHERE child.organisation_id = p_organisation_id
      AND parent.organisation_id = p_organisation_id
      AND child.key IN ('client_disability_standard', 'client_disability_emergency')
      AND parent.key = 'client_disability';

    FOR v_flow IN
        SELECT id, subject_kind
        FROM intake_flows
        WHERE organisation_id = p_organisation_id
    LOOP
        INSERT INTO intake_stages
            (id, organisation_id, flow_id, key, name, stage_kind, color, sort_order,
             is_terminal, on_enter_contact_status, sla_hours, checklist_gate,
             visible_fields, required_fields)
        VALUES
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'enquiry', 'Enquiry', 'enquiry', 'blue', 10,
             false, 'enquiry', 24, false,
             CASE WHEN v_flow.subject_kind = 'organisation'
                  THEN '["organisation_name","email","phone","notes"]'::jsonb
                  ELSE '["first_name","family_name","email","phone","date_of_birth"]'::jsonb
             END,
             CASE WHEN v_flow.subject_kind = 'organisation'
                  THEN '["organisation_name"]'::jsonb
                  ELSE '["first_name","family_name"]'::jsonb
             END),
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'eligibility', 'Eligibility / fit check', 'eligibility', 'teal', 20,
             false, 'onboarding', 48, true,
             CASE WHEN v_flow.subject_kind = 'organisation'
                  THEN '["email","phone","notes"]'::jsonb
                  ELSE '["date_of_birth","email","phone","funding","consent_services","consent_share"]'::jsonb
             END,
             CASE WHEN v_flow.subject_kind = 'organisation'
                  THEN '["email"]'::jsonb
                  ELSE '["date_of_birth","consent_services"]'::jsonb
             END),
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'risk_assessment', 'Risk assessment', 'risk_assessment', 'amber', 30,
             false, 'onboarding', 48, true,
             '["risk_summary","behaviour_notes"]'::jsonb,
             '["risk_summary"]'::jsonb),
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'service_planning', 'Service planning', 'service_planning', 'violet', 40,
             false, 'onboarding', 72, true,
             '["support_needs","funding","schedule_preferences","notes"]'::jsonb,
             '[]'::jsonb),
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'onboarded', 'Onboarded', 'onboarded', 'green', 50,
             true, 'active', NULL, false,
             '[]'::jsonb, '[]'::jsonb),
            (gen_random_uuid(), p_organisation_id, v_flow.id,
             'cancelled', 'Cancelled', 'cancelled', 'red', 60,
             true, 'intake_cancelled', NULL, false,
             '[]'::jsonb, '[]'::jsonb);

        INSERT INTO intake_stage_actions (organisation_id, stage_id, action_key, allowed)
        SELECT p_organisation_id, s.id, a.action_key,
               CASE s.stage_kind
                   WHEN 'eligibility' THEN a.action_key IN ('profile.edit', 'relationships.update')
                   WHEN 'risk_assessment' THEN a.action_key IN ('profile.edit', 'relationships.update')
                   WHEN 'service_planning' THEN a.action_key IN (
                       'profile.edit', 'relationships.update', 'funding.add', 'funding.edit'
                   )
                   WHEN 'onboarded' THEN true
                   ELSE false
               END
        FROM intake_stages s
        CROSS JOIN (VALUES
            ('roster.add'), ('roster.remove'),
            ('funding.add'), ('funding.edit'), ('funding.remove'),
            ('profile.edit'), ('relationships.update')
        ) AS a(action_key)
        WHERE s.organisation_id = p_organisation_id
          AND s.flow_id = v_flow.id;

        INSERT INTO intake_stage_checklist_items
            (id, organisation_id, stage_id, label, description, is_required, sort_order)
        SELECT gen_random_uuid(), p_organisation_id, s.id,
               v.label, v.description, v.is_required, v.sort_order
        FROM intake_stages s
        JOIN (VALUES
            ('enquiry', 'Record source', 'Where did this enquiry come from?', true, 10),
            ('enquiry', 'Confirm contact details', 'Name, phone and email are usable', true, 20),
            ('eligibility', 'Confirm identity', 'Photo ID or two secondary documents', true, 10),
            ('eligibility', 'Confirm funding and eligibility', 'Funding stream and fit confirmed', true, 20),
            ('eligibility', 'Record consent', 'Consent to collect and share information', true, 30),
            ('risk_assessment', 'Complete risk screen', 'Record known risks and mitigations', true, 10),
            ('risk_assessment', 'Record behaviour support needs', 'Note triggers and strategies', true, 20),
            ('risk_assessment', 'Assign risk owner', 'A named person owns the risk plan', true, 30),
            ('service_planning', 'Confirm services', 'Services and supports agreed', true, 10),
            ('service_planning', 'Confirm funding', 'Funding package or agreement in place', true, 20),
            ('service_planning', 'Confirm schedule preferences', 'Preferred days, times and workers', true, 30),
            ('service_planning', 'Prepare service agreement', 'Agreement ready for acceptance', true, 40),
            ('onboarded', 'Verify role and funding are active', 'Check contact role and funding status', true, 10),
            ('onboarded', 'Confirm first shift plan', 'First supports are scheduled', true, 20),
            ('cancelled', 'Record cancellation reason', 'Why the intake stopped', true, 10),
            ('cancelled', 'Notify referrer', 'Optional courtesy notification', false, 20)
        ) AS v(stage_kind, label, description, is_required, sort_order)
          ON v.stage_kind = s.stage_kind
        WHERE s.organisation_id = p_organisation_id
          AND s.flow_id = v_flow.id;
    END LOOP;
END;
$$;

CREATE OR REPLACE FUNCTION seed_intake_flows_for_organisation() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.tenant_id = NEW.id THEN
        PERFORM carelibre_seed_intake_flows(NEW.id);
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER organisations_seed_intake_flows
    AFTER INSERT ON organisations
    FOR EACH ROW EXECUTE FUNCTION seed_intake_flows_for_organisation();

SELECT carelibre_seed_intake_flows(id)
FROM organisations
WHERE tenant_id = id;

COMMENT ON FUNCTION carelibre_seed_intake_flows(uuid) IS
    'Seeds the default intake flows, stages, policies, checklists and actions for an organisation. Idempotent.';
