# Private local video review

This opt-in viewer combines an existing local MP4 library and, optionally, a
separately identified review-draft root. It does not copy, publish, upload,
register, or make external requests. Automated YouTube setup and public hosting
remain separate future work.

Run with Python 3 from the repository root. The default library is
`~/Desktop/chipviz`:

```sh
python3 scripts/local_review.py
```

To select an existing library and add a separate draft root:

```sh
python3 scripts/local_review.py --root /path/to/library --draft-root /path/to/drafts
```

The directories must exist and must not themselves be symlinks. `--port 0`
is the default and chooses an available port; `--port 8000` requests a
particular port. Open the printed URL on the same machine and stop the server
with Ctrl-C. Refresh the page to rescan both roots. Scanning hashes every MP4
and may take time. A malformed draft provenance file, inaccessible file, or
unsafe catalog entry fails the scan closed; fix the local input and refresh.
The viewer skips symlinks and never lists directories.

**Treat the entire URL as a password.** Anyone who obtains it on this machine
can view the selected footage. Do not share, screenshot, commit, or publish it.
Browser history, extensions, local users or malware may retain or access it.
The server binds only to `127.0.0.1`, requires a new random capability each
run, does not log requests, sends no-referrer and no-store headers, and makes
no third-party requests. These precautions do not protect against a compromised
machine or browser. Use trusted, stable directories and stop the server when
not in use.

Select a capture, open its embedded video, and seek using the player or the
time field. Use **Use current playback time** and **Copy reference** to copy
the identity, content hash, declared revision and timestamp without a URL,
token or file path. The stable identity incorporates the collection, relative
name and video bytes; renames or changed bytes produce a new identity. A
revision from a labelled library README or a draft `source_commit` is a
declaration, not independent proof. Other labels are inferred from filenames;
unknown information remains unknown. File changes detected between scanning
and playback are rejected until refresh. Changes during an active transfer
cannot be excluded, so do not edit footage while browsing.

Drafts are review-only and not human-reviewed. Hallway/House draft names identify
quiet or energetic scenarios and M1 or Pi 5 profiles; Pi-profile captures are
rendered on a Mac GPU, not physical Pi hardware. Normal-speed human review and
the existing safety/flash assessment remain required. This tool neither
performs nor attests to them. Retro captures are emulated, not hardware
validation. Some footage is restricted; local access is not permission to
publish it.

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
