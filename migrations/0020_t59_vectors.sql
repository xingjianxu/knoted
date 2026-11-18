-- Install in public so schema-isolated verification shares the extension.
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;

CREATE TABLE rag_embedding_sets (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    chunk_set_id uuid NOT NULL REFERENCES rag_chunk_sets(id) ON DELETE CASCADE,
    model text NOT NULL CHECK (length(model) BETWEEN 1 AND 128),
    model_revision text NOT NULL CHECK (length(model_revision) BETWEEN 1 AND 128),
    published_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (chunk_set_id, model, model_revision)
);

CREATE TABLE rag_vectors (
    embedding_set_id uuid NOT NULL REFERENCES rag_embedding_sets(id) ON DELETE CASCADE,
    chunk_id uuid NOT NULL REFERENCES rag_chunks(id) ON DELETE CASCADE,
    embedding vector(1024) NOT NULL,
    PRIMARY KEY (embedding_set_id, chunk_id)
);
CREATE INDEX rag_vectors_cosine_hnsw ON rag_vectors USING hnsw (embedding vector_cosine_ops);

-- Save every possibly submitted batch before contacting Python. On takeover
-- or explicit cancellation the original request can prove resource exit.
CREATE TABLE rag_embedding_executions (
    job_id uuid NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    attempt integer NOT NULL CHECK (attempt > 0),
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    request jsonb NOT NULL CHECK (jsonb_typeof(request) = 'object'),
    PRIMARY KEY (job_id, attempt, ordinal)
);
