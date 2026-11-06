CREATE TABLE tenants (id uuid PRIMARY KEY);
CREATE TABLE users (
    id uuid PRIMARY KEY,
    issuer text NOT NULL,
    subject text NOT NULL,
    UNIQUE (issuer, subject)
);
CREATE TABLE workspaces (
    id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    UNIQUE (tenant_id, id)
);
CREATE SEQUENCE membership_revision_seq;
CREATE TABLE workspace_memberships (
    user_id uuid NOT NULL REFERENCES users(id),
    tenant_id uuid NOT NULL,
    workspace_id uuid NOT NULL,
    role text NOT NULL CHECK (role IN ('owner', 'editor', 'viewer')),
    revision bigint NOT NULL DEFAULT nextval('membership_revision_seq'),
    PRIMARY KEY (user_id, tenant_id, workspace_id),
    FOREIGN KEY (tenant_id, workspace_id) REFERENCES workspaces(tenant_id, id)
);
CREATE TABLE personal_workspaces (
    user_id uuid PRIMARY KEY REFERENCES users(id),
    tenant_id uuid NOT NULL,
    workspace_id uuid NOT NULL UNIQUE,
    FOREIGN KEY (tenant_id, workspace_id) REFERENCES workspaces(tenant_id, id)
);
CREATE TABLE auth_sessions (
    token_hash bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    id uuid NOT NULL UNIQUE,
    user_id uuid NOT NULL REFERENCES users(id),
    tenant_id uuid NOT NULL REFERENCES tenants(id),
    csrf_hash bytea NOT NULL CHECK (octet_length(csrf_hash) = 32),
    expires_at timestamptz NOT NULL,
    revoked_at timestamptz
);
CREATE INDEX auth_sessions_user ON auth_sessions(user_id);
CREATE TABLE oidc_pending (
    state_hash bytea PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    browser_hash bytea NOT NULL CHECK (octet_length(browser_hash) = 32),
    issuer text NOT NULL,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    return_path text NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE TABLE control_audit (
    id bigserial PRIMARY KEY,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    event jsonb NOT NULL
);
