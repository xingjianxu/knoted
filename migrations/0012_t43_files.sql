-- ADR0006: identity registration is separate from unhashed observations.
CREATE TABLE file_roots (
 id uuid PRIMARY KEY,
 singleton boolean NOT NULL DEFAULT true UNIQUE CHECK(singleton),
 configured_path text NOT NULL,
 identity jsonb,
 generation bigint NOT NULL DEFAULT 0 CHECK(generation >= 0),
 scan_token uuid,
 last_complete_generation bigint,
 last_complete_at timestamptz,
 paused_reason text,
 created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE file_scans (
 id uuid PRIMARY KEY,
 root_id uuid NOT NULL REFERENCES file_roots(id),
 generation bigint NOT NULL CHECK(generation > 0),
 token uuid NOT NULL,
 job_id uuid NOT NULL REFERENCES jobs(id),
 lease_token uuid NOT NULL,
 started_at timestamptz NOT NULL DEFAULT now(),
 finished_at timestamptz,
 complete boolean NOT NULL DEFAULT false,
 error_class text,
 UNIQUE(root_id,generation),
 CHECK(NOT complete OR (finished_at IS NOT NULL AND error_class IS NULL))
);
CREATE TABLE file_observations (
 scan_id uuid NOT NULL REFERENCES file_scans(id),
 relative_path text NOT NULL CHECK(relative_path <> '' AND relative_path !~ '(^/|(^|/)\.\.(/|$))'),
 metadata jsonb NOT NULL,
 PRIMARY KEY(scan_id,relative_path)
);
CREATE TABLE file_sources (
 id uuid PRIMARY KEY REFERENCES entity_registry(id),
 root_id uuid NOT NULL REFERENCES file_roots(id),
 relative_path text NOT NULL CHECK(relative_path <> '' AND relative_path !~ '(^/|(^|/)\.\.(/|$))'),
 device bigint NOT NULL,
 inode bigint NOT NULL,
 size bigint NOT NULL CHECK(size >= 0),
 mtime_ns bigint NOT NULL,
 ctime_ns bigint NOT NULL,
 nlink bigint NOT NULL CHECK(nlink > 0),
 opaque_handle jsonb,
 sha256 text NOT NULL CHECK(sha256 ~ '^[0-9a-f]{64}$'),
 content_version bigint NOT NULL CHECK(content_version > 0),
 version bigint NOT NULL CHECK(version > 0),
 status text NOT NULL CHECK(status IN ('present','missing')),
 note text,
 mime text,
 -- Last observed metadata, independent of the captured/hash-matching values above.
 observation jsonb NOT NULL,
 observation_version bigint NOT NULL DEFAULT 1 CHECK(observation_version > 0),
 content_fresh boolean NOT NULL DEFAULT false,
 created_at timestamptz NOT NULL DEFAULT now(),
 updated_at timestamptz NOT NULL DEFAULT now(),
 last_seen_at timestamptz NOT NULL,
 last_seen_scan_id uuid REFERENCES file_scans(id)
);
CREATE UNIQUE INDEX file_sources_present_path ON file_sources(root_id,relative_path) WHERE status='present';
CREATE INDEX file_sources_scan ON file_sources(root_id,last_seen_scan_id);
