-- Snapshot the checklist item description when a case enters a stage.
--
-- The stage template already carries a description; keeping a copy on the case
-- item means later template edits do not rewrite the work already shown to
-- coordinators.

ALTER TABLE intake_case_checklist_items
    ADD COLUMN description text;

UPDATE intake_case_checklist_items ci
SET description = t.description
FROM intake_stage_checklist_items t
WHERE t.organisation_id = ci.organisation_id
  AND t.id = ci.template_item_id
  AND ci.description IS NULL;
