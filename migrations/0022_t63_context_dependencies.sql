CREATE TABLE rag_source_visibility (
    source_type text NOT NULL CHECK (source_type IN ('file','note','task')),
    source_id uuid NOT NULL,
    generation bigint NOT NULL CHECK (generation >= 0),
    PRIMARY KEY (source_type,source_id)
);

CREATE TABLE generation_source_dependencies (
    run_id uuid NOT NULL REFERENCES generation_runs(id) ON DELETE CASCADE,
    chunk_set_id uuid NOT NULL,
    source_type text NOT NULL CHECK (source_type IN ('file','note','task')),
    source_id uuid NOT NULL,
    source_version bigint NOT NULL CHECK (source_version > 0),
    visibility_generation bigint NOT NULL CHECK (visibility_generation >= 0),
    PRIMARY KEY (run_id,chunk_set_id)
);

CREATE FUNCTION rag_bump_visibility(kind text, identity uuid) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO rag_source_visibility (source_type,source_id,generation)
    VALUES (kind,identity,1)
    ON CONFLICT (source_type,source_id)
    DO UPDATE SET generation=rag_source_visibility.generation+1
$$;

CREATE FUNCTION rag_source_hidden() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM rag_bump_visibility(TG_ARGV[0],NEW.id);
    RETURN NEW;
END
$$;

CREATE TRIGGER rag_note_hidden AFTER UPDATE OF deleted_at,content_version ON notes
FOR EACH ROW WHEN ((OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
                   OR OLD.content_version<>NEW.content_version)
EXECUTE FUNCTION rag_source_hidden('note');

CREATE TRIGGER rag_task_hidden AFTER UPDATE OF deleted_at,version ON tasks
FOR EACH ROW WHEN ((OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
                   OR OLD.version<>NEW.version)
EXECUTE FUNCTION rag_source_hidden('task');

CREATE TRIGGER rag_file_hidden AFTER UPDATE OF status,content_version ON file_sources
FOR EACH ROW WHEN ((OLD.status='present' AND NEW.status='missing')
                   OR OLD.content_version<>NEW.content_version)
EXECUTE FUNCTION rag_source_hidden('file');

CREATE FUNCTION rag_project_tasks_hidden() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM rag_bump_visibility('task',id) FROM tasks WHERE project_id=NEW.id;
    RETURN NEW;
END
$$;

CREATE TRIGGER rag_project_tasks_hidden AFTER UPDATE OF deleted_at ON projects
FOR EACH ROW WHEN (OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL)
EXECUTE FUNCTION rag_project_tasks_hidden();

CREATE FUNCTION rag_task_activity_changed() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM rag_bump_visibility('task',COALESCE(NEW.task_id,OLD.task_id));
    RETURN NULL;
END
$$;

CREATE TRIGGER rag_activity_added AFTER INSERT ON activities
FOR EACH ROW WHEN (NEW.author_kind='manual')
EXECUTE FUNCTION rag_task_activity_changed();

CREATE TRIGGER rag_activity_updated AFTER UPDATE OF version ON activities
FOR EACH ROW WHEN (NEW.author_kind='manual' AND OLD.version<>NEW.version)
EXECUTE FUNCTION rag_task_activity_changed();

CREATE TRIGGER rag_activity_removed AFTER DELETE ON activities
FOR EACH ROW WHEN (OLD.author_kind='manual')
EXECUTE FUNCTION rag_task_activity_changed();
