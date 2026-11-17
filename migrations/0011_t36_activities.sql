-- T36: manual follow-up notes with history, plus append-only system status
-- events. Additive migration: no existing row is rewritten or removed.
CREATE TABLE activities (
    id uuid PRIMARY KEY,
    task_id uuid NOT NULL REFERENCES tasks(id),
    author_kind text NOT NULL CHECK (author_kind IN ('manual','system')),
    body text NOT NULL CHECK (char_length(body) BETWEEN 1 AND 10000),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    deleted_at timestamptz,
    -- A system event is never edited or removed; only a manual note is deleted
    -- and then keeps its marker row for history.
    CHECK (author_kind = 'manual' OR deleted_at IS NULL)
);
-- Follow-up pages are newest first within one task.
CREATE INDEX activities_page ON activities(task_id, created_at DESC, id DESC);

-- Content that is no longer current. A live note's current body stays in
-- `activities`; editing copies the superseded version here and deleting copies
-- the final version, so history keeps every former content without a second
-- mutable copy.
CREATE TABLE activity_revisions (
    id uuid PRIMARY KEY,
    activity_id uuid NOT NULL REFERENCES activities(id),
    version bigint NOT NULL CHECK (version >= 1),
    author_kind text NOT NULL CHECK (author_kind IN ('manual','system')),
    body text NOT NULL CHECK (char_length(body) BETWEEN 1 AND 10000),
    created_at timestamptz NOT NULL,
    UNIQUE (activity_id, version)
);
CREATE INDEX activity_revisions_page ON activity_revisions(activity_id, created_at DESC, id DESC);
