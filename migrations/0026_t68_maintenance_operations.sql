-- A committed marker survives loss of the advisory-lock connection. An
-- interrupted operation requires offline investigation before online backup.
CREATE TABLE maintenance_operations (
    id uuid PRIMARY KEY,
    backend_pid integer NOT NULL,
    started_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
