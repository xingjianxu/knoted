-- Successful complete artifacts only; failed/cancelled attempts remain in jobs.
CREATE TABLE file_extractions (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 source_id uuid NOT NULL REFERENCES file_sources(id),
 content_version bigint NOT NULL CHECK(content_version > 0),
 input_sha256 text NOT NULL CHECK(input_sha256 ~ '^[0-9a-f]{64}$'),
 parser_revision text NOT NULL,
 options_hash text NOT NULL CHECK(options_hash ~ '^[0-9a-f]{64}$'),
 job_id uuid NOT NULL REFERENCES jobs(id),
 execution_id uuid,
 blocks jsonb NOT NULL CHECK(jsonb_typeof(blocks)='array'),
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(source_id,content_version,parser_revision,options_hash)
);
