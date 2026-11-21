# Genesis hosted host-metadata preflight

This opt-in job collects limited host and daemon metadata only. It never uses
Docker or Buildx CLI, inspects containers, runs a container, builds anything,
or uploads an artifact. Its result **does not authorize toolchain builds**.
It does not alter the disabled #207 draft.

The path-scoped pull-request workflow runs only on a same-repository PR after
the `genesis-host-preflight-reviewed` label is added. A coordinator must first
obtain independent review of the exact PR head. A new head requires a fresh
review and a new label event. The job checks out that event's head SHA, with
read-only token permissions and no persisted checkout credentials. It does not
use secrets, external caches or release triggers. Its sanitized job log is
still sent to GitHub.

The helper only accepts the GitHub-hosted ARM64 environment and fixed
`/run/docker.sock`; it rejects inherited Docker routing variables. Before
connecting it checks root-owned socket and parent directories, an active
`docker.service`, the daemon executable and cgroup, and the listener inode in
the daemon's file descriptors. It then requires root `SO_PEERCRED` matching the
service PID and rechecks the socket and service around the single bounded
`GET /v1.48/info`. Systemd socket activation may cause `SO_PEERCRED` to name
the activator rather than dockerd; that ambiguity fails closed even when the
runner would otherwise be usable. The response has bounded framing, size and
time, and only fixed allowlisted fields are printed.

The 5 GiB free-space threshold on `/` and the expected daemon root is merely
a preflight minimum, not a proven build budget. A GitHub-hosted job is a fresh
VM according to the provider, but environment variables and these local checks
do not independently attest the provider, native hardware, exclusive daemon
access, storage or output-mount non-aliasing. Builder and mount isolation stay
`UNVERIFIED`; builds remain unauthorized. `GET /info` can update transient
daemon metrics; this is not a claim of literally zero daemon side effects.

Before any later work, review the actual job, socket/peer behavior, current
runner image and daemon implementation. #225 still needs a defensible builder
and output-mount isolation design; #207 still needs authenticated dependency
closure, isolated target builds, and review.

Sources: [GitHub-hosted runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners),
[Docker Engine info handler](https://github.com/moby/moby/blob/v28.0.4/daemon/info.go),
and [Unix peer credentials](https://man7.org/linux/man-pages/man7/unix.7.html).
