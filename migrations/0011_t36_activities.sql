-- T36: manual follow-up notes with history, plus append-only system status
-- events. Additive migration: no existing row is rewritten or removed.
CREATE TABLE activities (
    id uuid PRIMARY KEY,
    task_id uuid NOT NULL REFERENCES tasks(id),
    author_kind text NOT NULL CHECK (author_kind IN ('manual','system')),
    body text NOT NULL CHECK (char_length(body) <= 10000),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    deleted_at timestamptz,
    -- A system event is never edited or removed; only a manual note is deleted.
    CHECK (author_kind = 'manual' OR deleted_at IS NULL),
    -- A live note always carries text. Deleting moves the text to history and
    -- leaves an empty marker row, so the list can show that it was removed
    -- without exposing the former content outside history.
    CHECK (deleted_at IS NOT NULL OR char_length(body) >= 1)
);
-- Follow-up pages are newest first within one task.
CREATE INDEX activities_page ON activities(task_id, created_at DESC, id DESC);

-- Content that is no longer current. A live note's current body stays in
-- `activities`; editing and deleting copy the superseded version here, so
-- history keeps every former content (including a deleted note's last body)
-- without a second mutable copy.
CREATE TABLE activity_revisions (
    id uuid PRIMARY KEY,
    activity_id uuid NOT NULL REFERENCES activities(id),
    version bigint NOT NULL CHECK (version >= 1),
    author_kind text NOT NULL CHECK (author_kind IN ('manual','system')),
    body text NOT NULL CHECK (char_length(body) <= 10000),
    created_at timestamptz NOT NULL,
    UNIQUE (activity_id, version)
);
CREATE INDEX activity_revisions_page ON activity_revisions(activity_id, created_at DESC, id DESC);
