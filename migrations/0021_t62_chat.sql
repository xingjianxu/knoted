-- T62: durable dialogue, execution slots and ordered streaming events.
CREATE TABLE chat_sessions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    provider text NOT NULL CHECK (provider ~ '^[A-Za-z0-9][A-Za-z0-9_-]*$'),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    deleted_at timestamptz
);
CREATE INDEX chat_sessions_page ON chat_sessions(user_id, updated_at DESC, id DESC)
    WHERE deleted_at IS NULL;

CREATE TABLE chat_messages (
    id uuid PRIMARY KEY REFERENCES entity_registry(id),
    session_id uuid NOT NULL REFERENCES chat_sessions(id),
    role text NOT NULL CHECK (role IN ('user','assistant')),
    body text NOT NULL,
    incomplete boolean NOT NULL,
    run_id uuid,
    content_version bigint NOT NULL DEFAULT 1 CHECK (content_version > 0),
    ordinal bigint GENERATED ALWAYS AS IDENTITY,
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    deleted_at timestamptz,
    UNIQUE (id, session_id),
    CHECK (role <> 'user' OR (NOT incomplete AND run_id IS NULL))
);
CREATE INDEX chat_messages_order ON chat_messages(session_id, ordinal)
    WHERE deleted_at IS NULL;

CREATE TABLE generation_runs (
    id uuid PRIMARY KEY,
    session_id uuid NOT NULL REFERENCES chat_sessions(id),
    message_id uuid NOT NULL,
    assistant_message_id uuid NOT NULL,
    job_id uuid NOT NULL UNIQUE REFERENCES jobs(id),
    retry_of uuid REFERENCES generation_runs(id),
    provider text NOT NULL,
    model text,
    scope jsonb NOT NULL CHECK (jsonb_typeof(scope) = 'object'),
    state text NOT NULL CHECK (state IN (
        'queued','started','cancel_requested','completed','failed','cancelled','interrupted'
    )),
    admitted boolean NOT NULL,
    upstream_requested boolean NOT NULL DEFAULT false,
    partial boolean NOT NULL DEFAULT false,
    error text,
    last_seq bigint NOT NULL DEFAULT 0 CHECK (last_seq >= 0),
    created_at timestamptz NOT NULL,
    updated_at timestamptz NOT NULL,
    FOREIGN KEY (message_id, session_id) REFERENCES chat_messages(id, session_id),
    FOREIGN KEY (assistant_message_id, session_id) REFERENCES chat_messages(id, session_id),
    CHECK (message_id <> assistant_message_id),
    CHECK (NOT admitted OR state IN ('queued','started','cancel_requested','interrupted'))
);
ALTER TABLE chat_messages ADD FOREIGN KEY (run_id) REFERENCES generation_runs(id);
CREATE UNIQUE INDEX chat_one_admitted_run ON generation_runs (admitted) WHERE admitted;
CREATE UNIQUE INDEX chat_one_active_attempt ON generation_runs (message_id)
    WHERE state IN ('queued','started','cancel_requested') OR admitted;
CREATE INDEX chat_run_queue_order ON generation_runs(created_at, id) WHERE state = 'queued';

CREATE TABLE chat_run_events (
    run_id uuid NOT NULL REFERENCES generation_runs(id) ON DELETE CASCADE,
    seq bigint NOT NULL CHECK (seq > 0),
    type text NOT NULL CHECK (type IN ('queued','started','delta','citation','completed','failed','cancelled')),
    delta text,
    citation jsonb,
    error jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, seq),
    CHECK ((type = 'delta') = (delta IS NOT NULL)),
    CHECK ((type = 'citation') = (citation IS NOT NULL)),
    CHECK ((type = 'failed') = (error IS NOT NULL))
);

CREATE TABLE chat_submit_keys (
    user_id uuid NOT NULL REFERENCES users(id),
    key_sha256 bytea NOT NULL,
    request_sha256 bytea NOT NULL,
    run_id uuid NOT NULL REFERENCES generation_runs(id),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (user_id, key_sha256),
    CHECK (octet_length(key_sha256) = 32 AND octet_length(request_sha256) = 32)
);
CREATE INDEX chat_submit_keys_expiry ON chat_submit_keys(expires_at);
