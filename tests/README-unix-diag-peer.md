# Fake UNIX socket peer calibration

Run the focused standard-library tests from the repository root:

```sh
python3 -m unittest discover -s tests -p 'test_unix_diag_peer.py' -v
```

Decoder, evidence, socket-replacement, timeout and child-lifecycle tests run on
macOS and Linux. The three Linux diagnostic integration tests skip explicitly
on other platforms. They must pass on hosted Linux CI before merge; a
portable-only run is not Linux evidence. The path-scoped
`.github/workflows/unix-diag-peer.yml` job checks out the source and runs only
these verbose tests, with read-only permissions and no user-provided secrets,
cache, artifacts, or dependency installation.

The existing Python workflow runs broader checks and is not fake-only. The
repository's existing, unmodified PR checks run separately on GitHub-hosted
runners and include Docker, SDK, and ROM jobs. #244's no-Docker/SDK/ROM
restriction applies to the fixture, local work, and its dedicated job. No
local Docker access or suppression of standard checks is authorized. Those
checks do not prove receiver identity or isolation. Independent exact-head
review remains required before a PR.

The Linux fixture creates a private Unix stream listener and a disposable
child. Its private temporary directory is under `/tmp`, and socket paths are
checked against Darwin's shorter `sun_path` limit. In one case the parent
listens before passing the FD to the child; in the other, the child creates
its own listener. The client exchanges a
challenge with the child, checks `SO_PEERCRED` against the known listener
creator, and requests `UNIX_DIAG_PEER` for its own connected inode. It checks
that the known child's reported accepted FD is the **only** FD in that child
with the peer inode. After two observations, the child acknowledges a bounded
hold and rejects replacement commands. The parent then repeats the cookie,
peer, FD and client checks while the child remains held through return.
Replacement before the hold is rechecked; replacement through the cooperative
protocol during the hold is refused. Deadlines and child cleanup apply
throughout.

The test requests no netlink dump, does not read a global socket list, and
reads FD links only under `/proc/<fixture-child-pid>/fd`. The kernel may
search its internal socket tables while resolving an exact inode query. All
sockets and processes used by the fixture belong to the test.

This is calibration, not a production guard. The hold depends on the fixture
child's cooperation; a crash, timeout, release or out-of-band change can end
it, and observations can change after return. Observations are not atomic:
inodes can be reused, descriptors can be shared, and namespaces or permissions
can alter what is observable.
It does not establish a real daemon's identity, authorization, exclusive
ownership, or a trustworthy socket-activation chain for #226. It does not
establish daemon, builder, store, or mount isolation for #225 and does not
authorize a toolchain build. No live preflight is changed or relaxed.
