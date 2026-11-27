-- T53: unified searchable projection and the per-engine publication ledger.
--
-- Search projection for notes, projects and tasks. Each search hit is checked
-- against its current domain record. Exactly one version per source is current.
CREATE TABLE source_documents (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source_type text NOT NULL CHECK (source_type IN ('note', 'project', 'task')),
    source_id uuid NOT NULL,
    source_version bigint NOT NULL CHECK (source_version > 0),
    title text NOT NULL CHECK (title <> ''),
    body text NOT NULL,
    project_id uuid,
    tags text[] NOT NULL DEFAULT '{}',
    archived boolean NOT NULL DEFAULT false,
    deleted boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL,
    superseded_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source_type, source_id, source_version)
);

-- One current projection per source. Publishing an older version after a newer
-- one is rejected instead of silently replacing the current row.
CREATE UNIQUE INDEX source_documents_current_idx
    ON source_documents (source_type, source_id)
    WHERE superseded_at IS NULL;
CREATE INDEX source_documents_updated_idx ON source_documents (updated_at DESC, id);

-- One row per (engine, index, document key). The key is derived from the source
-- identity and version, so a retry upserts the same Meilisearch document instead
-- of creating a second visible hit. `published_at` is written in the same
-- transaction as `published`, so "published without a confirmation" cannot be
-- represented.
CREATE TABLE fulltext_publications (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id uuid NOT NULL REFERENCES source_documents (id) ON DELETE CASCADE,
    engine text NOT NULL CHECK (engine IN ('meilisearch')),
    index_uid text NOT NULL,
    document_key text NOT NULL CHECK (document_key ~ '^[a-z]+_[0-9a-f]{32}_v[0-9]+$'),
    status text NOT NULL DEFAULT 'pending' CHECK (
        status IN ('pending', 'submitted', 'published', 'superseded', 'failed')
    ),
    meili_task_uid bigint,
    job_id uuid REFERENCES jobs (id) ON DELETE SET NULL,
    attempts integer NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz,
    UNIQUE (engine, index_uid, document_key),
    CONSTRAINT fulltext_publications_published_check CHECK (
        status <> 'published' OR published_at IS NOT NULL
    )
);
CREATE INDEX fulltext_publications_document_idx ON fulltext_publications (document_id);
CREATE INDEX fulltext_publications_source_idx
    ON fulltext_publications (engine, index_uid, status);
