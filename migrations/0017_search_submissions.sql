-- A lost HTTP response cannot establish that Meilisearch rejected a write.
-- Keep the registration until a definitive response is known.
CREATE TABLE maintenance_external_submissions (
    id uuid PRIMARY KEY,
    index_uid text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Remember which destination may still be applying an acknowledged task.
CREATE TABLE maintenance_external_targets (
    origin text NOT NULL,
    index_uid text NOT NULL,
    PRIMARY KEY (origin, index_uid)
);
