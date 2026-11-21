# Private local video review

This opt-in viewer combines an existing local MP4 library and, optionally, a
separately identified review-draft root. It does not copy, publish, upload,
register, or make external requests. Automated YouTube setup and public hosting
remain separate future work.

The Python viewer targets macOS and Linux and relies on POSIX directory and
signal APIs; it does not support Windows. Run with Python 3 from the repository
root. The default library is
`~/Desktop/chipviz`:

```sh
python3 scripts/local_review.py
```

To select an existing library and add a separate draft root:

```sh
python3 scripts/local_review.py --root /path/to/library --draft-root /path/to/drafts
```

The directories must exist, must not themselves be symlinks, and must not
overlap: neither may contain the other. The check compares opened directory
identities and ancestry; use trusted, stable directories. Bind-mount aliases
and moves after startup may not be detected. `--port 0`
is the default and chooses an available port; `--port 8000` requests a
particular port. Open the printed URL on the same machine and stop the server
with Ctrl-C. Refresh the page to rescan both roots. Scanning hashes every MP4
and may take time. Each scan is limited to 50,000 directory entries, 32
directory levels below each root, 16 GiB of hashed video bytes, and 300 seconds
of elapsed work across both roots. Limits and shutdown cancellation are checked
incrementally; an uninterruptibly blocked filesystem syscall can still delay
shutdown beyond the elapsed limit. A limit breach, malformed draft provenance
file, inaccessible file, or unsafe catalog entry fails the scan closed; fix
the local input and refresh.
Unchanged indexed clips remain playable during a scan; a failed scan clears the
index. The viewer skips symlinks and never lists directories.

**Treat the entire URL as a password.** Anyone who obtains it on this machine
can view the selected footage. Do not share, screenshot, commit, or publish it.
Browser history, extensions, local users or malware may retain or access it.
The server binds only to `127.0.0.1`, requires a new random capability each
run, does not log requests, sends no-referrer and no-store headers, and makes
no third-party requests. These precautions do not protect against a compromised
machine or browser. Use trusted, stable directories and stop the server when
not in use.

The server allows at most 16 simultaneous connections and closes excess ones.
It requests a listen backlog of 32 connections; the OS may impose a lower
limit and reject bursts before the server can reply.
Request headers have a five-second total deadline and a 64 KiB limit; network
I/O has a ten-second inactivity timeout. An untrusted local process can still
temporarily exhaust the connection limit, and a slow authorized video transfer
can occupy a slot. An overlapping catalog refresh is rejected with HTTP 503
and `Retry-After: 1`; retry after the current scan finishes. Shutdown closes
connections and waits for active scans before releasing the library descriptors.

Select a capture, open its embedded video, and seek using the player or the
time field. Wait for finite video metadata before copying a typed time; times
beyond the video duration are rejected. **Use current playback time** also
waits for the initial seek to finish. If loading or seeking fails, reopen the
video. Use **Copy reference** to copy the identity, content hash, declared
revision and timestamp without a URL, token or file path. Recognized
visualization, platform, variant and scenario
labels are included only from a strict allowlist and are marked as inferred;
draft references also include a fixed review-only, not-human-reviewed,
do-not-publish warning. Other free-form metadata is omitted. The stable identity
incorporates the collection, relative name and video bytes; renames or changed
bytes produce a new identity. A revision from a labelled library README or a draft
`source_commit` is a declaration, not independent proof. Other labels are
inferred from paths; a library family or variant without an identifying label
remains unknown, as do library capture sources. Library lookup
skips unrelated READMEs and searches ancestors; invalid or conflicting labelled
revision declarations across the nearest directory's READMEs leave the revision
unknown. A declaration must contain a single full SHA, optionally backticked
and followed by punctuation. File changes detected between scanning and
playback are rejected until refresh. Changes during an active transfer cannot
be excluded, so do not edit footage while browsing.

Scenario filters group recognized quiet/energetic filename suffixes and the
modern library's `mellow-96` and `driving-126` tokens. Ambiguous or conflicting
recognized scenarios are unknown; other descriptive filenames retain their
inferred labels, which are not copied as free-form reference metadata.

Drafts are review-only and not human-reviewed. Recognized Hallway/House draft
names infer quiet or energetic scenarios and M1 or Pi 5 profiles; the Pi-profile
drafts are labelled as rendered on a Mac GPU, not physical Pi hardware.
Normal-speed human review and the existing safety/flash assessment remain
required. This tool neither performs nor attests to them. Library paths alone
do not establish emulation or hardware validation. Some footage is restricted;
local access is not permission to publish it.

The recognized draft layout is
`stream-{hallway,house}-{quiet,energetic}-{m1,pi5}/evaluation.mp4`, with a
sibling `provenance.json` containing a full `source_commit`. Other MP4s under
the draft root also appear, conservatively marked review-only; their labels
and revisions may be unknown. No command lines or paths from provenance are
shown.

Focused developer checks (Node.js is needed only for frontend checks):

```sh
python3 -m unittest discover -s tests -p 'test_local_review.py'
node --test scripts/local-review/review.test.mjs
python3 -m py_compile scripts/local_review.py tests/test_local_review.py
node --check scripts/local-review/app.mjs
node --check scripts/local-review/lib.mjs
node --check scripts/local-review/review.test.mjs
just python-lint
just docs-lint
```
