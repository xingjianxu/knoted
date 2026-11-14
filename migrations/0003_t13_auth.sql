-- T13: the single owner account, its Passkeys and the one-time CLI tokens.
--
-- Knoted has exactly one account and no public registration. `users.singleton`
-- plus `UNIQUE (singleton)` is what enforces that in the database: a second
-- insert conflicts instead of racing a read-then-write check in application
-- code. An operator creates the account with a CLI-issued bootstrap token and
-- regains access with a recovery token; both are stored as SHA-256 digests and
-- are valid for ten minutes and one consumption.
--
-- WebAuthn ceremonies, Passkeys and sessions are created here because the
-- account they belong to is defined here; T14/T15 populate them. `state` holds
-- the server-only webauthn-rs state (`danger-allow-state-serialisation`), and a
-- recovery ceremony is bound to the credential token that authorised it.
--
-- Managed by sqlx: applied once, tracked in `_sqlx_migrations`.
CREATE TABLE users (
    id uuid PRIMARY KEY,
    singleton boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT users_singleton_unique UNIQUE (singleton),
    -- With `UNIQUE` this allows only the value `true`, so at most one row can
    -- exist and it cannot be inserted as a non-singleton account.
    CONSTRAINT users_singleton_check CHECK (singleton)
);

-- Only the digest is stored; the plaintext token exists in the operator's
-- terminal and never in the database or the logs.
CREATE TABLE credential_tokens (
    id uuid PRIMARY KEY,
    purpose text NOT NULL,
    token_hash bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz NULL,
    CONSTRAINT credential_tokens_purpose_check CHECK (
        purpose IN ('bootstrap', 'recovery')
    ),
    CONSTRAINT credential_tokens_token_hash_unique UNIQUE (token_hash)
);

CREATE TABLE passkey_credentials (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Raw credential id bytes; the API exposes them as base64url.
    credential_id bytea NOT NULL,
    -- webauthn-rs `Passkey` serialised by serde: public key, counter, flags.
    passkey jsonb NOT NULL,
    transports text[] NOT NULL DEFAULT '{}',
    name text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_used_at timestamptz NULL,
    CONSTRAINT passkey_credentials_credential_id_unique UNIQUE (credential_id)
);

CREATE INDEX passkey_credentials_user_id_idx ON passkey_credentials (user_id);

CREATE TABLE ceremonies (
    id uuid PRIMARY KEY,
    purpose text NOT NULL,
    user_id uuid NULL REFERENCES users (id) ON DELETE CASCADE,
    credential_token_id uuid NULL REFERENCES credential_tokens (id) ON DELETE CASCADE,
    -- Server-only webauthn-rs state; never returned to the browser.
    state jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz NULL,
    CONSTRAINT ceremonies_purpose_check CHECK (
        purpose IN ('bootstrap', 'login', 'add_passkey', 'recovery')
    )
);

-- Deleting a credential token must not scan the whole ceremony table.
CREATE INDEX ceremonies_credential_token_id_idx ON ceremonies (credential_token_id);

CREATE TABLE sessions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash bytea NOT NULL,
    csrf_token_hash bytea NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    idle_expires_at timestamptz NOT NULL,
    absolute_expires_at timestamptz NOT NULL,
    revoked_at timestamptz NULL,
    last_verified_at timestamptz NULL,
    user_agent text NULL,
    CONSTRAINT sessions_token_hash_unique UNIQUE (token_hash)
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);
