-- T25: durable changed-only history. Existing notes become eligible for startup
-- recovery; no current note content/version is changed by this migration.
ALTER TABLE notes ADD COLUMN history_dirty_at timestamptz;
UPDATE notes SET history_dirty_at=updated_at;
CREATE INDEX notes_history_due ON notes(history_dirty_at, id) WHERE history_dirty_at IS NOT NULL;

CREATE TABLE note_snapshots (
    snapshot_id uuid PRIMARY KEY,
    note_id uuid NOT NULL REFERENCES notes(id),
    content_version bigint NOT NULL CHECK (content_version >= 1),
    title text NOT NULL,
    document jsonb NOT NULL,
    reason text NOT NULL CHECK (reason IN ('flush','periodic','recovery','before_restore')),
    created_at timestamptz NOT NULL,
    UNIQUE (note_id, content_version)
);
CREATE INDEX note_snapshots_page ON note_snapshots(note_id, created_at DESC, snapshot_id DESC);
CREATE INDEX note_snapshots_expiry ON note_snapshots(created_at);
