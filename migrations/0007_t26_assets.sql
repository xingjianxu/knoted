-- T26: private image storage and transactional current/history/trash references.
CREATE TABLE assets (
    id uuid PRIMARY KEY,
    owner_id uuid NOT NULL REFERENCES users(id),
    storage_key text NOT NULL UNIQUE CHECK (storage_key = id::text),
    mime text NOT NULL CHECK (mime IN ('image/png','image/jpeg','image/webp')),
    size bigint NOT NULL CHECK (size >= 0 AND size <= 20971520),
    sha256 text NOT NULL CHECK (sha256 ~ '^[a-f0-9]{64}$'),
    state text NOT NULL CHECK (state IN ('pending','ready','deleted')),
    created_at timestamptz NOT NULL,
    zero_ref_at timestamptz
);
CREATE TABLE asset_refs (
    asset_id uuid NOT NULL REFERENCES assets(id),
    ref_kind text NOT NULL CHECK (ref_kind IN ('current','snapshot','trash')),
    ref_id uuid NOT NULL,
    note_id uuid NOT NULL REFERENCES notes(id),
    PRIMARY KEY (asset_id,ref_kind,ref_id),
    CHECK (ref_kind='snapshot' OR ref_id=note_id)
);
CREATE INDEX asset_refs_source_idx ON asset_refs(ref_kind,ref_id);
CREATE INDEX asset_refs_note_idx ON asset_refs(note_id);
CREATE INDEX assets_zero_ref_idx ON assets(zero_ref_at) WHERE zero_ref_at IS NOT NULL;
CREATE TABLE asset_upload_requests (
    owner_id uuid NOT NULL REFERENCES users(id),
    operation text NOT NULL CHECK (operation='asset.upload'),
    key text NOT NULL CHECK (octet_length(key) BETWEEN 1 AND 255),
    request_digest text NOT NULL CHECK (request_digest ~ '^[a-f0-9]{64}$'),
    asset_id uuid NOT NULL REFERENCES assets(id),
    expires_at timestamptz NOT NULL,
    PRIMARY KEY (owner_id,operation,key)
);
CREATE INDEX asset_upload_requests_expiry_idx ON asset_upload_requests(expires_at);
