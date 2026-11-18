-- T57: preserve the immutable chunk identity independently of either engine.
CREATE TABLE rag_chunk_sets (
    id uuid PRIMARY KEY,
    source_type text NOT NULL CHECK (source_type IN ('file', 'note', 'task')),
    source_id uuid NOT NULL,
    source_version bigint NOT NULL CHECK (source_version > 0),
    extraction_id uuid NOT NULL,
    chunker_revision text NOT NULL CHECK (length(chunker_revision) BETWEEN 1 AND 128),
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source_type, source_id, source_version, extraction_id, chunker_revision)
);

CREATE TABLE rag_chunks (
    id uuid PRIMARY KEY,
    chunk_set_id uuid NOT NULL REFERENCES rag_chunk_sets(id) ON DELETE CASCADE,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    text text NOT NULL CHECK (text <> ''),
    token_count integer NOT NULL CHECK (token_count BETWEEN 1 AND 768),
    locations jsonb NOT NULL CHECK (jsonb_typeof(locations) = 'array'),
    UNIQUE (chunk_set_id, ordinal)
);

CREATE TABLE rag_chunk_publications (
    chunk_set_id uuid NOT NULL REFERENCES rag_chunk_sets(id) ON DELETE CASCADE,
    index_uid text NOT NULL,
    job_id uuid NOT NULL REFERENCES jobs(id),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'submitted', 'published', 'failed')),
    meili_task_uid bigint,
    last_confirmed_meili_task_uid bigint,
    next_ordinal integer NOT NULL DEFAULT 0 CHECK (next_ordinal >= 0),
    published_at timestamptz,
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (chunk_set_id, index_uid),
    CHECK ((status = 'submitted') = (meili_task_uid IS NOT NULL)),
    CHECK ((status = 'published') = (published_at IS NOT NULL))
);

-- Evidence readers compare the exact extraction identity as well as the source
-- version and the running chunker revision. Manual follow-up revisions are
-- independent of tasks.version, so include the ordered current activity IDs
-- and versions in the task extraction identity. System events are audit data.
CREATE VIEW rag_source_snapshots AS
SELECT 'file'::text AS source_type, f.id AS source_id, f.content_version AS source_version,
       e.id AS extraction_id, NULL::uuid AS project_id,
       (f.status = 'present' AND f.content_fresh) AS visible
FROM file_sources f
JOIN LATERAL (
    SELECT id FROM file_extractions
    WHERE source_id = f.id AND content_version = f.content_version AND input_sha256 = f.sha256
    ORDER BY created_at DESC, id DESC
    LIMIT 1
) e ON true

UNION ALL

SELECT 'note', n.id, n.content_version, md5(concat('note:', n.id, ':', n.content_version))::uuid, NULL::uuid, n.deleted_at IS NULL
FROM notes n

UNION ALL

SELECT 'task', t.id, t.version,
       md5(concat('task:', t.id, ':', t.version, ':', a.revisions))::uuid, t.project_id, (t.deleted_at IS NULL AND p.deleted_at IS NULL)
FROM tasks t
LEFT JOIN projects p ON p.id = t.project_id
LEFT JOIN LATERAL (
    SELECT string_agg(concat(id, ':', version), ',' ORDER BY id) AS revisions
    FROM activities WHERE task_id = t.id AND author_kind = 'manual'
) a ON true;

CREATE VIEW current_rag_sources AS
SELECT source_type, source_id, source_version, extraction_id, project_id
FROM rag_source_snapshots WHERE visible;
