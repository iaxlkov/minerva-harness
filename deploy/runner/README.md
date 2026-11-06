# Private Linux runner

`cargo build --release -p minerva-runner` produces the private supervisor. Run
`minerva-runner /etc/minerva/runner.json` as a trusted, dedicated Linux supervisor
with a private bidirectional pipe to the control plane. Do not expose stdin/stdout
as a public API. The runner is privileged infrastructure: Docker socket access
is equivalent to host administrative access. Only this supervisor receives the
socket; worker Hosts and user execution containers never receive it.

No commands in this document have been applied to the developer machine.
The example config is deliberately unusable until an operator supplies both
existing, approved image digests. The runner never pulls images and never falls
back from runsc to runc or a local process.

## Operator prerequisites

- Linux x86_64, Docker with the independently installed and registered gVisor
  `runsc` runtime. Independently verify the runtime registration points to the
  intended runsc binary. Docker `info` must report Linux and runsc.
- A dedicated Btrfs filesystem with quotas enabled by the operator, mounted at
  the absolute `root` path. Use root ownership and mode 0700; do not share this
  directory with another runner. No symlink component is accepted. Runner checks
  the filesystem and creates only tenant/workspace UUID descendants, then creates
  a Btrfs subvolume and enforces its qgroup hard limit before mounting anything.
  Missing Btrfs, disabled quotas, command failures, and unsupported filesystems
  fail closed. Install `btrfs`, `chown`, `chmod`, and `docker` on the fixed system
  PATH. No storage pool or runtime is automatically installed or reconfigured.
- A pre-created Docker **internal bridge** named `minerva-control-*`. Only
  platform-approved gateway/control components and worker Hosts may join it.
  The runner validates internal+bridge, never creates/reconfigures networks, and
  never publishes worker ports. The gateway-to-worker authenticated bridge and
  PostgreSQL lease/heartbeat provisioning are separate control-plane integration.
- Pinned worker and execution images already loaded into Docker. Images must
  support UID/GID 65532 and a read-only root filesystem. The worker entrypoint
  runs only the trusted Host service, listens on its private control network,
  stores SQLite/history at `/data`, configuration at `/config`, and treats
  `/workspace` as workspace files. It must send user commands through the
  execution provider, never `docker exec` or a local worker shell. The execution
  image needs a suitable shell/coreutils for the opt-in test; normal commands
  are explicit argument vectors subject to the approved image entrypoint.
- Private runner control state at `root/.runner-control`; it must survive runner
  restart and never be restored independently from assignment ownership state.
  The file lock permits one runner per storage root. Cross-node fencing and
  lease authority must be enforced by the control plane; two different storage
  roots are not a distributed lock.

## Boundaries and limits

Every container uses `--runtime=runsc`, drops all capabilities, disables privilege
escalation, uses UID 65532, a read-only root, private PID/network namespaces,
no inherited platform environment, and bounded CPU, RAM+swap, PIDs, file handles,
logs and tmpfs. No privileged/host network/host PID option is accepted. The only
extra execution environment keys are LANG, LC_ALL and TERM. Docker itself is
invoked using argument arrays, a fixed unix socket, a cleared environment, fixed
PATH, null stdin and bounded output/control-operation time.

The persistent workspace/data/config share one Btrfs subvolume, bounded to
`floor(diskMiB*2/3)` MiB. Host and the single serialized execution each get at most
`floor(diskMiB/6)` MiB tmpfs, so their payload storage stays within diskMiB.
Approved image layers, Docker metadata, and at most 1 MiB of logs per container
are infrastructure overhead outside this payload budget. RAM also limits tmpfs.
Execution containers mount only `/workspace`; they cannot mount Host data,
config, keys, SQLite, the runner journal, another workspace, or the Docker socket.
Data/config and UUID parent directories use private modes. No paths or mounts
are accepted from the control request.

Execution network is always **none**. This version intentionally has no scoped
egress implementation: a request cannot enable egress, metadata access, or a
control network. An egress broker and endpoint policy require a separately
reviewed implementation before outbound access can be enabled. Host networking
is restricted to the trusted internal control bridge.

## JSON-line RunnerControl v1

One UTF-8 JSON object per line, maximum 1 MiB. Unknown fields are rejected.
Accepted requests complete in order. Invalid or excess ordinary requests receive
an immediate error and are never queued, so these rejection responses can precede
the outstanding operation's response. Exactly one response is emitted per input
line. Stdout is reserved
for protocol responses and stderr for supervisor failure diagnostics. An
oversized/incomplete frame closes the connection. Keep the private control pipe
open: EOF cancels outstanding execution but leaves long-lived Hosts running.

Start:

```json
{"op":"start","assignment":{"tenantId":"11111111-1111-4111-8111-111111111111","workspaceId":"22222222-2222-4222-8222-222222222222","assignmentId":"33333333-3333-4333-8333-333333333333","workerId":"44444444-4444-4444-8444-444444444444","generation":7},"limits":{"memoryMiB":2048,"cpuMilli":1000,"pids":256,"diskMiB":10240}}
```

Stop/status use `{"op":"stop","assignment":{...}}` and
`{"op":"status","assignment":{...}}`. Execute uses:

```json
{"op":"execute","assignment":{"tenantId":"11111111-1111-4111-8111-111111111111","workspaceId":"22222222-2222-4222-8222-222222222222","assignmentId":"33333333-3333-4333-8333-333333333333","workerId":"44444444-4444-4444-8444-444444444444","generation":7},"command":{"argv":["/bin/sh","-c","printf hello"],"env":{"LANG":"C.UTF-8"},"timeoutMs":5000}}
```

Success is `{"ok":true,"result":{"assignment":{...},"running":true}}`.
Execute includes `output:{exitCode,stdout,stderr,timedOut}`. Each output stream
is capped at 1 MiB. Timeout gives exitCode -1 and timedOut true. Explicit stop
cancellation gives exitCode -1 and timedOut false, followed by the stop response.
Errors are `{"ok":false,"error":"..."}`. Error messages do not contain command
output or inherited secrets. No `isolationVerified` or hosted-ready claim is
provided by this service.

The control reader can accept a matching stop immediately after execute and
sets an atomic cancellation flag while execution is running. Only one normal outstanding request and one stop are admitted. Extra normal
requests are rejected immediately with `runner busy`; they never occupy a queue
slot or prevent the reader from accepting stop. A matching stop cancels the active
execution token before it is queued, even if another stop is already pending. The serialized supervisor checks exact tenant,
workspace, assignment, worker and generation identity. It stops **all** labelled
containers and confirms absence before a replacement mount. Generation is
journaled and synced before provisioning; failed starts burn their generation.
Restart stops all journalled workspace containers, marks assignments inactive,
and requires a newer generation. A failed stop prevents every new mount.
The control plane must derive these IDs from authorized assignment state, not
browser DTOs. They match `minerva-worker-auth::AssertionClaims`; delivering the
assignment-scoped derived assertion key to the Host is the control-plane bridge's
responsibility, not a browser-controlled environment option.

## Verification and acceptance

Portable controlled tests:

```
cargo test -p minerva-execution -p minerva-runner
```

On a dedicated provisioned Linux test machine, with test-only operator config:

```
MINERVA_RUNSC_CONFIG=/etc/minerva/test-runner.json MINERVA_RUNSC_TEST_OPT_IN=yes cargo test -p minerva-execution --test linux_runsc -- --ignored --nocapture
```

This opt-in test creates random tenant/workspace UUID subvolumes, runs actual
runsc commands, checks unavailable private/neighbor mounts and symlink targets,
no execution interface, and an orphan process that must not survive timeout.
It leaves only its test UUID subvolumes for operator inspection; remove only the
UUID directories created by that test after confirming no matching labelled
containers remain. Never point the test at working user data.

This smoke test alone is not full Gate B acceptance. Real fork/resource exhaustion,
DNS/redirect/metadata attack probes, two-user availability, worker/tool
compatibility, assignment lease/gateway crash integration, and the full hosted
browser suite still need a Linux deployment. Docker Desktop daemon was unavailable
in the current Windows environment; **no real sandbox acceptance is claimed**.
