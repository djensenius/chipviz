# Fake UNIX socket peer calibration

Run the focused standard-library tests from the repository root:

```sh
python3 -m unittest discover -s tests -p 'test_unix_diag_peer.py' -v
```

Decoder, evidence, socket-replacement, timeout and child-lifecycle tests run on
macOS and Linux. The three Linux diagnostic integration tests skip explicitly
on other platforms. They must pass on hosted Linux CI before merge; a
portable-only run is not Linux evidence. The existing Python workflow runs
these tests through unittest discovery.

The Linux fixture creates a private Unix stream listener and a disposable
child. In one case the parent listens before passing the FD to the child; in
the other, the child creates its own listener. The client exchanges a
challenge with the child, checks `SO_PEERCRED` against the known listener
creator, and requests `UNIX_DIAG_PEER` for its own connected inode. It checks
that the known child's reported accepted FD is the **only** FD in that child
with the peer inode. A second exact-inode request uses the first socket cookie
and repeats the FD check. Deadlines, bounded reads, and child cleanup apply
throughout; a replacement-FD race must fail closed.

The test requests no netlink dump, does not read a global socket list, and
reads FD links only under `/proc/<fixture-child-pid>/fd`. The kernel may
search its internal socket tables while resolving an exact inode query. All
sockets and processes used by the fixture belong to the test.

This is calibration, not a production guard. Observations are not atomic:
processes and FDs can change between checks, inodes can be reused, descriptors
can be shared, and namespaces or permissions can alter what is observable.
It does not establish a real daemon's identity, authorization, exclusive
ownership, or a trustworthy socket-activation chain for #226. It does not
establish daemon, builder, store, or mount isolation for #225 and does not
authorize a toolchain build. No live preflight is changed or relaxed.
