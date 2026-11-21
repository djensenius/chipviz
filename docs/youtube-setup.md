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
3. In your own Python environment, install the dependencies from
   `scripts/requirements-youtube.txt`. Run from the repository root:

   ```sh
   python3 scripts/youtube_auth.py configure
   python3 scripts/youtube_auth.py authorize
   ```

   Paste the ID and secret only into the non-echoing local prompts. The second
   command opens a browser for Google consent and asks for upload and read-only
   YouTube access. Check the displayed channel title and type its **exact
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
consent is not a guarantee of permanent unattended operation. If authorization
fails or is revoked, delete the private token file yourself and repeat
`authorize`. To replace the client, remove both private files yourself and
repeat both commands. Protect the old files when removing them. The uploader
must stop if `load_credentials()` fails, and must verify it is operating on the
returned expected channel before uploading. The loader refreshes with Google
each time, so it requires network access and rejects revoked or unavailable
authorization rather than treating stale credentials as valid.
Refresh-token rotation is saved atomically, including subsequent automatic
refreshes on the returned credentials. A process crash or storage failure after
Google rotates a token but before it is saved can require reauthorization.
Concurrent cooperating processes serialize using `.youtube.lock`; do not remove
or replace this lock while chipviz is running. Stale credentials must be
reloaded if another process rotates the token. A crash may leave an owner-only
`.youtube-<random>` temporary file in the app directory; it may contain a
secret and can be removed after all chipviz processes have stopped.

The importable interface is `load_credentials() -> tuple[object, str]`: a
refreshed Google Credentials object and the confirmed channel ID. It raises
`AuthError` on missing, unsafe, invalid, revoked, or unavailable authorization.
Call it serially: while contacting Google, it temporarily suppresses
process-wide logging and redirects standard output and error to protect OAuth
data from library diagnostics.
