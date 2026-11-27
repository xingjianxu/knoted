-- T05: shared entity registry.
--
-- Every entity that can be linked, indexed or cited gets one row here so that
-- identifiers stay meaningful across modules. Later task cards add their own
-- tables (notes, files, projects, tasks, chat messages) and register each new
-- row in this table; this migration intentionally creates no other tables.
--
-- Managed by sqlx: applied once, tracked in `_sqlx_migrations`.
CREATE TABLE entity_registry (
    id uuid PRIMARY KEY,
    entity_type text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT entity_registry_entity_type_check CHECK (
        entity_type IN ('note', 'file', 'project', 'task')
    )
);
