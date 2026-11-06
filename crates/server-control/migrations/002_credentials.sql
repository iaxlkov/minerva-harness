CREATE TABLE IF NOT EXISTS workspace_credentials (
 tenant_id uuid NOT NULL,
 workspace_id uuid NOT NULL,
 reference text NOT NULL CHECK (length(reference) BETWEEN 1 AND 256),
 version bigint NOT NULL CHECK (version > 0),
 key_version integer NOT NULL CHECK (key_version > 0),
 nonce bytea NOT NULL CHECK (octet_length(nonce) = 12),
 ciphertext bytea NOT NULL,
 PRIMARY KEY (tenant_id, workspace_id, reference),
 FOREIGN KEY (tenant_id, workspace_id) REFERENCES workspaces(tenant_id, id) ON DELETE CASCADE
);
