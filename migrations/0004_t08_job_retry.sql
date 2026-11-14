-- T08: manual retries point at the job they repeat.
--
-- An automatic retry reuses the same row (it stays `retry_wait` until its next
-- claim), while a manual retry is a new job: the original row keeps its terminal
-- state and the new row records which job it was created from.
--
-- Managed by sqlx: applied once, tracked in `_sqlx_migrations`.
ALTER TABLE jobs ADD COLUMN retry_of uuid NULL REFERENCES jobs (id);

-- The index supports listing the retries of one job. Partial, because the vast
-- majority of jobs are not manual retries.
CREATE INDEX jobs_retry_of_idx ON jobs (retry_of) WHERE retry_of IS NOT NULL;
