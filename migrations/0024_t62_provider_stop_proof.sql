-- Freeze the identity of the upstream instance before sending generation work.
-- A legacy request without identity cannot borrow a stop report after restart.
ALTER TABLE generation_runs
    ADD COLUMN provider_stop_identity text CHECK (provider_stop_identity ~ '^[0-9a-f]{64}$'),
    ADD COLUMN upstream_stopped boolean NOT NULL DEFAULT false,
    ADD CHECK (NOT upstream_stopped OR upstream_requested);
