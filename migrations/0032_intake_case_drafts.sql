-- New intake records start as drafts so details can be captured in the record.
--
-- A draft case can be discarded before its first field save. The linked
-- placeholder contact is removed with it; saving the case promotes the contact
-- from the case data and clears the draft flag.

ALTER TABLE intake_cases
    ADD COLUMN is_draft boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN intake_cases.is_draft IS
    'True until the first field save promotes the case and its placeholder contact.';
