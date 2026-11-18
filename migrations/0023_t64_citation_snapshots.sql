CREATE TABLE generation_citation_snapshots (
    id uuid PRIMARY KEY,
    run_id uuid NOT NULL REFERENCES generation_runs(id) ON DELETE CASCADE,
    message_id uuid NOT NULL REFERENCES chat_messages(id),
    chunk_set_id uuid NOT NULL,
    source_type text NOT NULL CHECK (source_type IN ('note','file','task')),
    source_id uuid NOT NULL,
    source_version bigint NOT NULL CHECK (source_version > 0),
    visibility_generation bigint NOT NULL CHECK (visibility_generation >= 0),
    title text NOT NULL,
    location text,
    snippet text NOT NULL,
    ordinal integer NOT NULL CHECK (ordinal >= 0),
    UNIQUE (run_id, ordinal),
    FOREIGN KEY (run_id,chunk_set_id)
        REFERENCES generation_source_dependencies(run_id,chunk_set_id) ON DELETE CASCADE
);

CREATE INDEX generation_citations_message
    ON generation_citation_snapshots(message_id,ordinal,id);
