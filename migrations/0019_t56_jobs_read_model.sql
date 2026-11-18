-- Start time was not previously stored. Preserve NULL for existing attempts
-- and record the first claim for jobs starting after this migration.
ALTER TABLE jobs ADD COLUMN started_at timestamptz;

CREATE INDEX jobs_created_page_idx ON jobs (created_at DESC, id DESC);
CREATE INDEX jobs_state_created_page_idx ON jobs (state, created_at DESC, id DESC);
