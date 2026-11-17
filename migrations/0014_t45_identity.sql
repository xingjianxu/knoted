-- Only sources made absent by this complete scan may be matched as moves.
-- A later scan clears this transient eligibility; historical missing stays missing.
ALTER TABLE file_sources ADD COLUMN move_scan_id uuid REFERENCES file_scans(id);
CREATE INDEX file_sources_move_scan ON file_sources(move_scan_id) WHERE move_scan_id IS NOT NULL;
