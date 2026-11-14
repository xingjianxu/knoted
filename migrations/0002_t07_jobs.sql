-- T07: durable job queue, leases and the transactional outbox.
--
-- A job is one unit of background work (parse, embed, index, ...). Workers
-- claim a job with a short transaction, receive a `lease_token`, keep the lease
-- alive with heartbeats and may only commit a result while the token still
-- matches. `attempts` counts claims, so an expired lease that is taken over
-- increments it; the retry/backoff policy on top of this is T08.
--
-- The outbox row is written in the same transaction as the job row, so a
-- consumer never observes an event for a job that was rolled back.
--
-- Managed by sqlx: applied once, tracked in `_sqlx_migrations`.
CREATE TABLE jobs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    kind text NOT NULL,
    idempotency_key text,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    state text NOT NULL DEFAULT 'queued',
    priority integer NOT NULL DEFAULT 0,
    attempts integer NOT NULL DEFAULT 0,
    max_attempts integer NOT NULL DEFAULT 4,
    available_at timestamptz NOT NULL DEFAULT now(),
    lease_token uuid,
    lease_expires_at timestamptz,
    locked_by text,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT jobs_state_check CHECK (
        state IN (
            'queued',
            'running',
            'retry_wait',
            'succeeded',
            'failed',
            'cancel_requested',
            'cancelled'
        )
    ),
    CONSTRAINT jobs_attempts_check CHECK (attempts >= 0),
    CONSTRAINT jobs_max_attempts_check CHECK (max_attempts >= 1),
    -- A lease exists exactly while a worker owns the job: `running` and
    -- `cancel_requested` both hold it, so a cancellation request keeps the
    -- resources and the execution quota until the worker confirms (matching the
    -- RAG rule that no resource is released without a cancel confirmation).
    -- Every other state must have all three lease columns NULL, which forces the
    -- state machine to release the lease in the same statement that ends a run.
    CONSTRAINT jobs_lease_check CHECK (
        (state IN ('running', 'cancel_requested')
            AND lease_token IS NOT NULL
            AND lease_expires_at IS NOT NULL
            AND locked_by IS NOT NULL)
        OR (state NOT IN ('running', 'cancel_requested')
            AND lease_token IS NULL
            AND lease_expires_at IS NULL
            AND locked_by IS NULL)
    )
);

-- Enqueueing is idempotent per (kind, key): the duplicate insert loses the
-- conflict and the existing job is returned instead of failing. Jobs without a
-- key are never deduplicated, which is why the index is partial.
CREATE UNIQUE INDEX jobs_kind_idempotency_key_idx
    ON jobs (kind, idempotency_key)
    WHERE idempotency_key IS NOT NULL;

-- Claim scan: due queued jobs, plus running jobs whose lease expired. The
-- order matches the claim query (`priority DESC, available_at ASC`) so the
-- scan does not sort the whole candidate set.
CREATE INDEX jobs_claim_order_idx ON jobs (state, priority DESC, available_at);
CREATE INDEX jobs_lease_expires_at_idx
    ON jobs (lease_expires_at)
    WHERE lease_expires_at IS NOT NULL;

CREATE TABLE outbox (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    job_id uuid REFERENCES jobs (id) ON DELETE CASCADE,
    topic text NOT NULL,
    payload jsonb NOT NULL DEFAULT '{}'::jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    published_at timestamptz
);

CREATE INDEX outbox_unpublished_idx
    ON outbox (created_at)
    WHERE published_at IS NULL;
