-- Closed input objects are internal capture state, not a second attempt ledger.
ALTER TABLE file_sources
 ADD COLUMN input_object_id text CHECK(input_object_id ~ '^[A-Za-z0-9_-]+$'),
 ADD COLUMN captured_observation jsonb,
 ADD COLUMN captured_observation_version bigint,
 ADD COLUMN captured_scan_id uuid REFERENCES file_scans(id);
ALTER TABLE file_sources ADD CONSTRAINT file_capture_complete CHECK (
 (input_object_id IS NULL AND captured_observation IS NULL AND captured_observation_version IS NULL AND captured_scan_id IS NULL)
 OR (input_object_id IS NOT NULL AND captured_observation IS NOT NULL AND captured_observation_version IS NOT NULL AND captured_observation_version > 0 AND captured_scan_id IS NOT NULL)
);
