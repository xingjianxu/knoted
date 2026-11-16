-- T23: current note contents. History, assets and tree move/restore are later cards.
-- Additive migration: no existing data is rewritten or removed.
CREATE TABLE notes (
    id uuid PRIMARY KEY REFERENCES entity_registry(id),
    parent_id uuid REFERENCES notes(id),
    title text NOT NULL CHECK (char_length(title) <= 500),
    document jsonb NOT NULL,
    format_version integer NOT NULL DEFAULT 1 CHECK (format_version = 1),
    content_version bigint NOT NULL DEFAULT 1 CHECK (content_version >= 1),
    position bigint NOT NULL CHECK (position >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    deleted_at timestamptz,
    CHECK (parent_id IS DISTINCT FROM id)
);
-- Structural writes take the notes tree transaction lock before checking a
-- parent or assigning/repacking positions; this also serializes root appends.
CREATE INDEX notes_live_siblings ON notes(parent_id, position, id) WHERE deleted_at IS NULL;
CREATE INDEX notes_live_list ON notes(position, id) WHERE deleted_at IS NULL;
