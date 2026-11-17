-- T33: projects and the minimal task relation/lifecycle contract extended by T34.
-- No physical cascades: entity identities and links survive soft deletion.
CREATE TABLE projects (
    id uuid PRIMARY KEY REFERENCES entity_registry(id),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 500),
    description text,
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    archived_at timestamptz,
    deleted_at timestamptz,
    archive_batch_id uuid,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);
CREATE INDEX projects_page ON projects(created_at, id) WHERE deleted_at IS NULL;

CREATE TABLE tasks (
    id uuid PRIMARY KEY REFERENCES entity_registry(id),
    project_id uuid REFERENCES projects(id),
    status text NOT NULL DEFAULT 'todo' CHECK (status IN ('todo','in_progress','done','cancelled')),
    version bigint NOT NULL DEFAULT 1 CHECK (version >= 1),
    archived_at timestamptz,
    deleted_at timestamptz,
    archive_batch_id uuid,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL
);
CREATE INDEX tasks_project ON tasks(project_id, id);
