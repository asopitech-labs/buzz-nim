-- Nim-owned workflow lifecycle decisions need a durable revision and bounded,
-- idempotent transition identity on the run row they mutate.
ALTER TABLE workflow_runs
    ADD COLUMN revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    ADD COLUMN transition_ids JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(transition_ids) = 'array');
