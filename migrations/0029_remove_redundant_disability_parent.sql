-- Remove the redundant generic disability flow.
--
-- `client_disability` duplicated the standard six-stage pipeline and was also
-- acting as a parent for the standard and emergency disability variants. The
-- variants are now top-level flows, so the parent has no purpose.

UPDATE intake_flows child
SET parent_flow_id = NULL
FROM intake_flows parent
WHERE child.organisation_id = parent.organisation_id
  AND child.parent_flow_id = parent.id
  AND parent.key = 'client_disability';

-- Pre-release installations may have already seeded it. Delete it when no case
-- references it; otherwise archive it rather than destroying history.
DELETE FROM intake_flows flow
WHERE flow.key = 'client_disability'
  AND NOT EXISTS (
      SELECT 1 FROM intake_cases c WHERE c.flow_id = flow.id
  );

UPDATE intake_flows
SET is_active = false
WHERE key = 'client_disability';

-- Give the two disability variants distinct grouping values so they can each
-- have their own default without colliding with the generic standard flow.
UPDATE intake_flows
SET variant = 'disability_standard'
WHERE key = 'client_disability_standard';

UPDATE intake_flows
SET variant = 'disability_emergency'
WHERE key = 'client_disability_emergency';

-- Future organisations seed eight flows directly. Replace the seed function so
-- it no longer recreates the parent.

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
        (gen_random_uuid(), p_organisation_id, 'client_disability_standard',
         'Client — disability standard', 'client', 'disability_standard',
         'Standard disability intake', 30),
        (gen_random_uuid(), p_organisation_id, 'client_disability_emergency',
         'Client — disability emergency', 'client', 'disability_emergency',
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
