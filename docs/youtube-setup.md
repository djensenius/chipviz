# YouTube authorization setup

This is a one-time, local setup for a dedicated Google Cloud project. The
publisher is responsible for making uploads unlisted; OAuth itself does not
set video privacy.

1. Create a dedicated project in Google Cloud and enable the YouTube Data
   API v3. In Google Auth Platform, choose the Personal audience if offered.
   Otherwise select the audience appropriate to your account and complete
   Google's consent and test-user configuration. Availability and verification
   requirements are determined by Google.
2. Create an OAuth client ID of type **Desktop app**. Keep the client ID and
   client secret private. Do not copy them into chat, the repository, shell
   history, or command-line arguments.
3. In your own terminal, prepare a virtual environment and run the setup from
   the repository root. Use the mise-managed Python (3.10 or newer); macOS's
   built-in Python 3.9 cannot import the upload transport. Git must be
   installed, because setup verifies the private storage is outside any Git
   worktree.

   ```sh
   mise exec -- python -m venv .venv
   .venv/bin/python -m pip install -r scripts/requirements-youtube.txt
   .venv/bin/python scripts/youtube_auth.py configure
   .venv/bin/python scripts/youtube_auth.py authorize
   ```

   Paste the ID and secret only into the non-echoing local prompts. The second
   command opens a browser for Google consent and asks for upload and read-only
   YouTube access. Run it on the Mac's own desktop session: the authorization
   URL is intentionally not printed, so if no browser opens, press Control-C
   (or wait for the five-minute timeout) and retry from a local session. Leading
   or trailing whitespace in pasted values is ignored. Check the displayed channel title and type its **exact
   channel ID** to confirm. Do not confirm an unexpected channel.

The configuration lives in `~/.config/chipviz/.env.youtube` and the refresh
token and expected channel ID in `~/.config/chipviz/youtube-token.json`,
outside Git. The app directory must be owned by you and mode 0700, and both
files must be owned by you and mode 0600. Configuration is never overwritten.
Storage on macOS and Linux is supported. On macOS, ancestor directories may
have deny-only ACLs; other ACLs are rejected. The app directory, private files,
and lock file must have no ACLs on either platform.
Never commit, share, or back up these files to an untrusted location. Other
processes running as your user, administrators, malware, and insecure backups
can still read or alter them. This is not a hardware-backed secret store.

Google can revoke refresh tokens, and testing-mode or unused tokens can expire;
consent is not a guarantee of permanent unattended operation. After a network,
storage, or other uncertain failure, stop and preserve all private state; do not
delete it or start another upload. A failed or interrupted token rotation may
leave the current token unknown and requires deliberate recovery. Only after
definitive revocation and an intentional recovery decision should you securely
preserve the existing private state and remove the token file to repeat
`authorize`. Replacing the client is a separate intentional operation: securely
preserve both private files before removing them and repeating both commands.
The uploader must stop if `load_credentials()` fails, and must verify it is
operating on the returned expected channel before uploading. The loader
refreshes with Google each time, so it requires network access and rejects
revoked or unavailable authorization rather than treating stale credentials as
valid.
Refresh-token rotation is saved atomically, including subsequent automatic
refreshes on the returned credentials. A process crash or storage failure after
Google rotates a token but before it is saved has an uncertain outcome; preserve
the state and quarantine the operation rather than automatically reauthorizing.
Concurrent cooperating processes serialize using `.youtube.lock`; do not remove
or replace this lock while chipviz is running. After a confirmed successful
rotation by another process, stale credentials must be reloaded; an uncertain
rotation instead requires quarantine. A crash may leave an owner-only
`.youtube-<random>` temporary file in the app directory; it may contain the
only copy of a rotated secret. Preserve it securely for deliberate recovery.

The importable interface is `load_credentials() -> tuple[object, str]`: a
refreshed Google Credentials object and the confirmed channel ID. It raises
`AuthError` on missing, unsafe, invalid, revoked, or unavailable authorization.
`AuthError.kind` is `auth_required` only for a definitive authorization failure;
`uncertain` requires preserving state and stopping.
Token exchanges use only Google's fixed HTTPS token endpoint, without proxies,
redirects, or automatic retries.
Call it serially: while contacting Google, it temporarily suppresses
process-wide logging and redirects standard output and error to protect OAuth
data from library diagnostics.
