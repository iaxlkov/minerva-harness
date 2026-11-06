# Hosted gateway

This binary is the public control plane. It has no local Runtime and never starts a shared workspace host. HTTPS termination must sit in front of the loopback listener and preserve the public Host header. Unavailable identity/database/runner services fail closed.

## Configuration

| Environment | Meaning |
| --- | --- |
| `MINERVA_PUBLIC_ORIGIN` | Required canonical HTTPS origin, without path/query/credentials |
| `MINERVA_CONTROL_DATABASE_URL` | Required PostgreSQL DSN; the gateway enforces certificate-verified TLS |
| `MINERVA_OIDC_ISSUER` | Required administrator-configured HTTPS OIDC issuer |
| `MINERVA_OIDC_CLIENT_ID` | Required OIDC client ID |
| `MINERVA_OIDC_CLIENT_SECRET` | Optional confidential-client secret |
| `MINERVA_RUNNER_ORIGIN` | Required configured HTTPS runner reverse proxy origin |
| `MINERVA_WORKER_ASSERTION_KEY` | Required base64 master key, at least 32 random bytes; gateway and trusted runner only |
| `MINERVA_ALLOW_ENROLLMENT` | `true` admits new verified identities; default `false` |
| `MINERVA_SESSION_SECONDS` | Session lifetime in seconds, 1 through 86400; default 43200 |
| `MINERVA_WEB_DIST` | Built shared frontend directory; default `dist` |
| `MINERVA_GATEWAY_BIND` | Private listener; default `127.0.0.1:4320` |

Register the exact callback `${MINERVA_PUBLIC_ORIGIN}/auth/callback` at the OIDC provider. Configure trusted certificates for PostgreSQL, the external issuer and runner HTTPS endpoints. There is no certificate verification bypass or HTTP worker fallback.

Run `cargo run -p minerva-gateway`. Startup migrates the control schema and `workspace_worker_assignments`, discovers the OIDC issuer and binds the gateway listener. Runner availability is checked per assignment and readiness probe; startup does not fabricate assignments.

## Browser routes

An unauthenticated SPA document request redirects to `/auth/login?returnTo=...`; shared static assets can load publicly. Login uses authorization code, PKCE, state and nonce. Callback verifies identity, transactionally provisions its personal tenant/workspace, and stores a bounded persistent session. The browser receives opaque `__Host-minerva_session`, `__Host-minerva_csrf` cookies with Secure/HttpOnly/SameSite=Lax/Path=/ attributes. Provider tokens are never returned.

`GET /api/v1/bootstrap` selects the user's personal workspace. An optional `workspaceId` query selects only an existing current membership within the authenticated tenant. Bootstrap validates the CSRF cookie against the current persistent session before returning the synchronizer token. The client keeps it in memory and sends it in `X-Minerva-CSRF`; no browser bearer token belongs in localStorage.

`POST /api/v1/rpc` requires exact Origin, current session, session-bound CSRF matching the HttpOnly CSRF cookie and `X-Minerva-Workspace` from bootstrap. The wire remains `{protocolVersion:1,method,params}`. The registry includes all 22 compatibility calls, `runs.start/get/cancel`, and `plugins.call`. User/tenant/workspace/assignment/worker-address overrides in params are rejected. Workspace UUIDs and object UUIDs never choose a worker endpoint. All session/run/plugin object keys are resolved in that one workspace worker's isolated store. Owner/editor/viewer decisions happen before forwarding.

`GET /api/v1/streams?workspaceId=...` requires exact Origin and membership. Gateway bridges the open/close and baseline/event/resync stream framing. Every opened subscription names a session in the selected worker; unknown frames, unsolicited IDs, mismatched outer/inner session IDs, malformed payloads, noncontiguous or unsafe cursors, and deliveries for closed scopes close the connection. Only session.updated, message.upsert and run.updated events are accepted; resync pauses events until a fresh nonregressing baseline. Membership, session and active assignment are revalidated before each data delivery; an idle timer and revocation notifications also close access. A tab's former connection cannot receive another workspace's events.

`POST /auth/logout` and `/api/v1/logout` require Origin and CSRF, revoke the persistent session and clear all session/CSRF/login cookies. APIs return JSON failures and never redirect to HTML. Hosted `codex_login` is explicitly unavailable; bootstrap does not advertise that capability.

## Trusted runner and worker protocol

The runner proxy exposes `GET /health/ready` with `{ready:true,sandbox:"runsc",isolationVerified:true}` only after actual sandbox readiness. Worker routes sit at `${MINERVA_RUNNER_ORIGIN}/workers/${assignment_id}`; the proxy strips this prefix before the private workspace Host. The configured runner origin, exact assignment prefix, active row, generation, heartbeat within ten seconds and unexpired lease are mandatory. A missing runner, assignment, fresh heartbeat, sandbox verification or matching lease yields 503. Platform/runner code owns assignment lifecycle updates; no browser endpoint writes this table.

`workspace_worker_assignments` has a workspace primary key, composite tenant/workspace FK, unique assignment ID, worker UUID, generation > 0, endpoint, runsc profile, status, lease and heartbeat. Generation/lease ownership and stop-before-remount must be enforced by the runner. An assignment generation must change when replacing its worker process.

Private RPC remains `POST /api/v1/rpc` with the original exact request bytes. Private WebSocket remains `GET /api/v1/streams` with an empty body. Only the generated WebSocket handshake headers or JSON Content-Type plus `X-Minerva-Assertion` are forwarded; browser cookies, auth headers and scope headers are not forwarded.

`minerva-worker-auth` specifies `base64url(JSON claims).base64url(HMAC-SHA256(payload bytes))`. This is a short-lived internal assertion, not a JWT implementation. Claims bind version, assignment, generation, worker audience, user, tenant, workspace, exact action/HTTP method/path, SHA-256 body hash, issue/expiry and request UUID. Gateway uses a 30-second lifetime. Worker verification permits at most 60 seconds.

Gateway/runner derive the per-worker key with `AssertionKey::derive_worker(assignment_id,generation,worker_id,tenant_id,workspace_id)`. Only that derived key may be mounted in the worker; the master key must never enter tenant execution. Worker verifies against its configured assignment fields and actual HTTP/body/method, then consumes `ReplayCache::admit(&verified, now)` before dispatch. The worker cache holds at most 4096 unexpired request IDs, rejects duplicates/full/future/expired values and never evicts a live entry. Private WebSocket authorization expires at its assertion expiry; browser reconnection obtains a fresh assertion through the gateway.

## Verification limits

`cargo test -p minerva-gateway -p minerva-worker-auth` runs hermetic boundary/policy/stream/signature/key-isolation/replay tests. The ignored PostgreSQL assignment constraint test requires a disposable `MINERVA_TEST_POSTGRES_URL`; never use a working deployment database for tests.

A real PostgreSQL migration run, full browser OIDC exchange, proxy-to-runsc worker routing and two-user Linux sandbox acceptance have not been demonstrated by these unit tests. Public hosted readiness remains contingent on the complete deployment gate. The directory's runtime readiness checks do not replace those acceptance results.
