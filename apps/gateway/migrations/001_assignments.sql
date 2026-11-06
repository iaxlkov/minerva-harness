CREATE TABLE workspace_worker_assignments (
    workspace_id uuid PRIMARY KEY,
    tenant_id uuid NOT NULL,
    assignment_id uuid NOT NULL UNIQUE,
    worker_id uuid NOT NULL,
    generation bigint NOT NULL CHECK (generation > 0),
    endpoint text NOT NULL,
    sandbox_profile text NOT NULL CHECK (sandbox_profile = 'runsc'),
    status text NOT NULL CHECK (status IN ('starting','ready','stopping','stopped','failed')),
    lease_until timestamptz NOT NULL,
    healthy_at timestamptz,
    FOREIGN KEY (tenant_id, workspace_id) REFERENCES workspaces(tenant_id, id)
);
