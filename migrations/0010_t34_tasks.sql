-- No invented titles: an unknown pre-T34 row must abort this upgrade unchanged.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM tasks) THEN
        RAISE EXCEPTION 'T34 requires an empty tasks table or an explicitly reviewed title mapping';
    END IF;
END $$;
ALTER TABLE tasks
    ADD COLUMN title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 500),
    ADD COLUMN description text,
    ADD COLUMN parent_id uuid REFERENCES tasks(id),
    ADD CONSTRAINT tasks_parent_not_self CHECK (parent_id IS DISTINCT FROM id),
    ADD COLUMN priority text NOT NULL DEFAULT 'none' CHECK (priority IN ('none','low','medium','high')),
    ADD COLUMN "start" jsonb,
    ADD COLUMN due jsonb;
CREATE INDEX tasks_parent ON tasks(parent_id, id);
CREATE INDEX tasks_page ON tasks(created_at, id) WHERE deleted_at IS NULL;
