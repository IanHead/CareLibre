-- Keep the tenant column when an optional waitlist reference is cleared.
--
-- `ON DELETE SET NULL` on a composite foreign key nulls every referencing
-- column, including `organisation_id`. That column is NOT NULL, so deleting a
-- division or service with a waitlist entry failed. Clear only the optional
-- division/service reference and leave the tenant intact.

ALTER TABLE intake_waitlist_entries
    DROP CONSTRAINT intake_waitlist_service_fk,
    DROP CONSTRAINT intake_waitlist_division_fk;

ALTER TABLE intake_waitlist_entries
    ADD CONSTRAINT intake_waitlist_service_fk
        FOREIGN KEY (organisation_id, service_id)
        REFERENCES services (organisation_id, id)
        ON DELETE SET NULL (service_id),
    ADD CONSTRAINT intake_waitlist_division_fk
        FOREIGN KEY (organisation_id, division_id)
        REFERENCES divisions (organisation_id, id)
        ON DELETE SET NULL (division_id);
