-- A client cancellation or lost HTTP response cannot establish that the remote
-- service rejected a write. Keep the registration until its receipt is known.
CREATE TABLE maintenance_external_submissions (
    id uuid PRIMARY KEY,
    service text NOT NULL CHECK (service IN ('meili', 'python')),
    index_uid text,
    execution_request jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((service = 'meili') = (index_uid IS NOT NULL)),
    CHECK ((service = 'python') = (execution_request IS NOT NULL))
);

-- Remember which external destination remains responsible after a registration
-- is acknowledged, including after a configuration or generation change.
CREATE TABLE maintenance_external_targets (
    service text NOT NULL CHECK (service IN ('meili','python')),
    origin text NOT NULL,
    index_uid text NOT NULL,
    PRIMARY KEY (service, origin, index_uid),
    CHECK ((service='python') = (index_uid=''))
);
