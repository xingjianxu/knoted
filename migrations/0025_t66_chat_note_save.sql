CREATE TABLE chat_note_save_keys (
    user_id uuid NOT NULL REFERENCES users(id),
    key_sha256 bytea NOT NULL CHECK (octet_length(key_sha256) = 32),
    request_sha256 bytea NOT NULL CHECK (octet_length(request_sha256) = 32),
    note_id uuid NOT NULL REFERENCES notes(id),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (user_id, key_sha256)
);
