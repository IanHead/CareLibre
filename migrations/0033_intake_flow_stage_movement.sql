-- Let each intake flow choose between free stage movement and a sequential pipeline.
--
-- Free movement keeps the current behaviour: any non-terminal stage can move to
-- any other stage. Sequential movement exposes only the previous and next stage
-- and enforces that rule on the server.

ALTER TABLE intake_flows
    ADD COLUMN stage_movement text NOT NULL DEFAULT 'any'
        CHECK (stage_movement IN ('any', 'sequential'));

COMMENT ON COLUMN intake_flows.stage_movement IS
    'any = free movement between stages; sequential = previous/next stage only.';
