#!/usr/bin/env python3
"""Local, private YouTube installed-app authorization for chipviz."""

from __future__ import annotations

import argparse
import contextlib
import ctypes
import errno
import fcntl
import getpass
import io
import json
import logging
import os
from pathlib import Path
import re
import secrets
import stat
import subprocess
import sys
import warnings


SCOPES = (
  "https://www.googleapis.com/auth/youtube.upload",
  "https://www.googleapis.com/auth/youtube.readonly",
)
AUTH_URI = "https://accounts.google.com/o/oauth2/auth"
TOKEN_URI = "https://oauth2.googleapis.com/token"
ENV_FILE = ".env.youtube"
STATE_FILE = "youtube-token.json"
LOCK_FILE = ".youtube.lock"
CLIENT_ID = re.compile(r"[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*\.apps\.googleusercontent\.com\Z")
CLIENT_SECRET = re.compile(r"[A-Za-z0-9_-]{8,256}\Z")
CHANNEL_ID = re.compile(r"UC[A-Za-z0-9_-]{22}\Z")


class AuthError(Exception):
  """A non-secret diagnostic; only auth_required warrants reauthorization."""

  def __init__(self, message, *, kind="uncertain"):
    super().__init__(message)
    self.kind = kind


@contextlib.contextmanager
def _quiet_google():
  # Library handlers may point to the original streams, not redirected ones.
  previous = logging.root.manager.disable
  logging.disable(max(previous, logging.CRITICAL))
  try:
    with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
      yield
  finally:
    logging.disable(previous)


def _check_acl(fd: int, *, allow_denies: bool = False) -> None:
  """Allow only verified Darwin deny entries on non-private ancestors."""
  if sys.platform == "darwin":
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    libc.acl_get_fd_np.argtypes = [ctypes.c_int, ctypes.c_int]
    libc.acl_get_fd_np.restype = ctypes.c_void_p
    libc.acl_free.argtypes = [ctypes.c_void_p]
    libc.acl_valid.argtypes = [ctypes.c_void_p]
    libc.acl_get_entry.argtypes = [ctypes.c_void_p, ctypes.c_int,
                                   ctypes.POINTER(ctypes.c_void_p)]
    libc.acl_get_tag_type.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_int)]
    acl = libc.acl_get_fd_np(fd, 0x100)  # ACL_TYPE_EXTENDED
    if acl:
      try:
        if not allow_denies or libc.acl_valid(acl) != 0:
          raise AuthError("Unsafe access control list.")
        entry = ctypes.c_void_p()
        # Darwin's ACL_LAST_ENTRY is declared in sys/acl.h. Compare entry
        # descriptors so an enumeration error cannot masquerade as end-of-ACL.
        last = ctypes.c_void_p()
        if libc.acl_get_entry(acl, -2, ctypes.byref(last)) != 0 or not last.value:
          raise AuthError("Cannot verify access control list.")
        index = 0  # ACL_FIRST_ENTRY
        for _ in range(1024):
          if libc.acl_get_entry(acl, index, ctypes.byref(entry)) != 0 or not entry.value:
            raise AuthError("Cannot verify access control list.")
          tag = ctypes.c_int()
          if libc.acl_get_tag_type(entry, ctypes.byref(tag)) != 0 or tag.value != 2:
            raise AuthError("Unsafe access control list.")
          if entry.value == last.value:
            ctypes.set_errno(0)
            if libc.acl_get_entry(acl, -1, ctypes.byref(entry)) != -1 or ctypes.get_errno() != errno.EINVAL:
              raise AuthError("Cannot verify access control list.")
            break
          index = -1  # ACL_NEXT_ENTRY
        else:
          raise AuthError("Cannot verify access control list.")
      finally:
        libc.acl_free(acl)
      return
    if ctypes.get_errno() != errno.ENOENT:
      raise AuthError("Cannot verify private storage access controls.")
  elif sys.platform.startswith("linux"):
    try:
      os.getxattr(fd, "system.posix_acl_access")
    except OSError as error:
      if error.errno != errno.ENODATA:
        raise AuthError("Cannot verify private storage access controls.") from None
    else:
      raise AuthError("Access control lists are not supported for private storage.")
  else:
    raise AuthError("Cannot verify private storage access controls on this platform.")


def _directory(create: bool) -> int:
  """Open the private directory without following any path symlinks."""
  home = Path.home()
  if not home.is_absolute() or ".." in home.parts:
    raise AuthError("Unsafe home directory.")
  parts = (*home.parts[1:], ".config", "chipviz")
  fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
  try:
    for index, part in enumerate(parts):
      try:
        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
      except FileNotFoundError:
        if not create or index < len(parts) - 2:
          raise AuthError("Private directory is missing.") from None
        os.mkdir(part, 0o700, dir_fd=fd)
        child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
      os.close(fd)
      fd = child
      info = os.fstat(fd)
      is_app = index == len(parts) - 1
      _check_acl(fd, allow_denies=not is_app)
      is_home_or_config = index >= len(parts) - 3
      sticky_root = info.st_uid == 0 and bool(info.st_mode & stat.S_ISVTX)
      if (info.st_uid not in (0, os.getuid())
          or (is_home_or_config and info.st_uid != os.getuid())
          or (info.st_mode & 0o022 and not sticky_root)
          or (is_app and stat.S_IMODE(info.st_mode) != 0o700)):
        raise AuthError("Unsafe directory ownership or permissions.")
      try:
        os.stat(".git", dir_fd=fd, follow_symlinks=False)
      except FileNotFoundError:
        pass
      else:
        raise AuthError("Private storage must be outside any Git worktree.")
    # Also ask Git, including unusual worktree configurations.
    env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
    env.update(GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
    result = subprocess.run(
      ["git", "-C", str(home / ".config" / "chipviz"),
       "rev-parse", "--is-inside-work-tree"],
      env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
    )
    if result.returncode != 128:
      raise AuthError("Private storage must be outside any Git worktree.")
    return fd
  except Exception:
    os.close(fd)
    raise


def _read(fd: int, name: str) -> str:
  file_fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
  try:
    info = os.fstat(file_fd)
    _check_acl(file_fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
        or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1
        or info.st_size > 16384):
      raise AuthError("Unsafe private file.")
    with os.fdopen(file_fd, "r", encoding="utf-8") as file:
      file_fd = -1
      return file.read(16385)
  finally:
    if file_fd != -1:
      os.close(file_fd)


def _rename_noreplace(fd: int, source: str, target: str) -> None:
  """Use the platform's atomic no-replace rename, or fail closed."""
  if sys.platform == "darwin":
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    rename = libc.renameatx_np
    flags = 0x4  # RENAME_EXCL
  elif sys.platform.startswith("linux"):
    libc = ctypes.CDLL(None, use_errno=True)
    rename = libc.renameat2
    flags = 1  # RENAME_NOREPLACE
  else:
    raise AuthError("Atomic private file publication is unavailable.")
  rename.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int,
                     ctypes.c_char_p, ctypes.c_uint]
  rename.restype = ctypes.c_int
  if rename(fd, os.fsencode(source), fd, os.fsencode(target), flags) != 0:
    error = ctypes.get_errno()
    raise OSError(error, os.strerror(error))


def _atomic_write(fd: int, name: str, content: str, *, replace: bool = False) -> None:
  """Publish one complete, single-link private file atomically."""
  temporary = ".youtube-" + secrets.token_hex(16)
  file_fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                    0o600, dir_fd=fd)
  try:
    os.fchmod(file_fd, 0o600)
    _check_acl(file_fd)
    with os.fdopen(file_fd, "w", encoding="utf-8") as file:
      file_fd = -1
      file.write(content)
      file.flush()
      os.fsync(file.fileno())
    if replace:
      os.replace(temporary, name, src_dir_fd=fd, dst_dir_fd=fd)
    else:
      _rename_noreplace(fd, temporary, name)
    os.fsync(fd)
  finally:
    if file_fd != -1:
      os.close(file_fd)
    try:
      os.unlink(temporary, dir_fd=fd)
    except FileNotFoundError:
      pass


def _write_once(fd: int, name: str, content: str) -> None:
  _atomic_write(fd, name, content)


@contextlib.contextmanager
def _state_lock(fd: int):
  """A stable advisory lock shared by all cooperating state operations."""
  try:
    lock_fd = os.open(LOCK_FILE, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                      0o600, dir_fd=fd)
    created = True
  except FileExistsError:
    lock_fd = os.open(LOCK_FILE, os.O_RDWR | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=fd)
    created = False
  try:
    if created:
      os.fchmod(lock_fd, 0o600)
    info = os.fstat(lock_fd)
    _check_acl(lock_fd)
    if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
        or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1):
      raise AuthError("Unsafe authorization lock.")
    fcntl.flock(lock_fd, fcntl.LOCK_EX)
    current = os.stat(LOCK_FILE, dir_fd=fd, follow_symlinks=False)
    if (current.st_dev, current.st_ino) != (info.st_dev, info.st_ino):
      raise AuthError("Authorization lock changed.")
    yield
  finally:
    os.close(lock_fd)


def _client_values(text: str) -> tuple[str, str]:
  lines = text.splitlines()
  if len(lines) != 2 or not lines[0].startswith("YOUTUBE_CLIENT_ID=") or not lines[1].startswith(
      "YOUTUBE_CLIENT_SECRET="):
    raise AuthError("Malformed private configuration.")
  client_id = lines[0].partition("=")[2]
  client_secret = lines[1].partition("=")[2]
  if not CLIENT_ID.fullmatch(client_id) or not CLIENT_SECRET.fullmatch(client_secret):
    raise AuthError("Malformed private configuration.")
  return client_id, client_secret


def _configure() -> None:
  """Prompt without echo and create the private configuration once."""
  fd = _directory(create=True)
  try:
    try:
      os.stat(ENV_FILE, dir_fd=fd, follow_symlinks=False)
    except FileNotFoundError:
      pass
    else:
      raise AuthError("Private configuration already exists.")
    with warnings.catch_warnings():
      warnings.simplefilter("error", getpass.GetPassWarning)
      client_id = getpass.getpass("Desktop OAuth client ID: ")
      client_secret = getpass.getpass("Desktop OAuth client secret: ")
    text = f"YOUTUBE_CLIENT_ID={client_id}\nYOUTUBE_CLIENT_SECRET={client_secret}\n"
    _client_values(text)
    _write_once(fd, ENV_FILE, text)
  finally:
    os.close(fd)


def _token_valid(token: object) -> bool:
  return isinstance(token, str) and bool(token) and len(token) <= 8192 and token.isprintable()


def _state_values(text: str) -> tuple[str, str]:
  state = json.loads(text)
  if not isinstance(state, dict) or set(state) != {"channel_id", "refresh_token"}:
    raise AuthError("Invalid authorization state.")
  channel_id, token = state["channel_id"], state["refresh_token"]
  if not isinstance(channel_id, str) or not CHANNEL_ID.fullmatch(channel_id) or not _token_valid(token):
    raise AuthError("Invalid authorization state.")
  return channel_id, token


def _state_text(channel_id: str, token: str) -> str:
  return json.dumps({"channel_id": channel_id, "refresh_token": token}) + "\n"


def _authorize_locked(fd: int) -> None:
  client_id, client_secret = _client_values(_read(fd, ENV_FILE))
  try:
    os.stat(STATE_FILE, dir_fd=fd, follow_symlinks=False)
  except FileNotFoundError:
    pass
  else:
    raise AuthError("Authorization already exists.")
  from google_auth_oauthlib.flow import InstalledAppFlow
  from googleapiclient.discovery import build

  client_config = {"installed": {
    "client_id": client_id, "client_secret": client_secret,
    "auth_uri": AUTH_URI, "token_uri": TOKEN_URI,
    "redirect_uris": ["http://localhost"],
  }}
  print("Opening the browser for YouTube authorization.")
  # The library can print an authorization URL or loopback request; neither
  # belongs in terminal logs.
  with _quiet_google():
    flow = InstalledAppFlow.from_client_config(
      client_config, scopes=SCOPES, autogenerate_code_verifier=True)
    credentials = flow.run_local_server(
      host="127.0.0.1", port=0, open_browser=True,
      authorization_prompt_message=None, timeout_seconds=300, prompt="consent")
    if not _token_valid(credentials.refresh_token):
      raise AuthError("Google did not provide a durable refresh token.")
    granted = credentials.granted_scopes
    if granted is not None and not set(SCOPES).issubset(granted):
      raise AuthError("Google did not grant both required scopes.")
    response = build("youtube", "v3", credentials=credentials, cache_discovery=False).channels().list(
      part="snippet", mine=True, maxResults=2).execute()
  items = response.get("items", [])
  if (not isinstance(items, list) or len(items) != 1 or response.get("nextPageToken")
      or response.get("pageInfo", {}).get("totalResults", 1) != 1):
    raise AuthError("YouTube channel is missing or ambiguous.")
  channel_id = items[0].get("id")
  title = items[0].get("snippet", {}).get("title")
  if (not isinstance(channel_id, str) or not CHANNEL_ID.fullmatch(channel_id)
      or not isinstance(title, str) or not title or len(title) > 200
      or not title.isprintable()):
    raise AuthError("YouTube channel is invalid.")
  print(f"Resolved channel: {title} ({channel_id})")
  if input("Type the exact channel ID to confirm: ") != channel_id:
    raise AuthError("Channel was not confirmed.")
  if not _token_valid(credentials.refresh_token):
    raise AuthError("Google did not provide a durable refresh token.")
  _write_once(fd, STATE_FILE, _state_text(channel_id, credentials.refresh_token))


def _authorize() -> None:
  fd = _directory(create=False)
  try:
    with _state_lock(fd):
      _authorize_locked(fd)
  finally:
    os.close(fd)


def configure() -> None:
  """Prompt without echo and create the private configuration once."""
  try:
    _configure()
  except Exception:
    raise AuthError("Cannot create private YouTube configuration.") from None


def authorize() -> None:
  """Authorize and confirm a single channel, without exposing library errors."""
  try:
    _authorize()
  except Exception:
    raise AuthError("Cannot authorize and confirm the YouTube channel.") from None


def load_credentials() -> tuple[object, str]:
  """Return credentials and channel; AuthError.kind distinguishes uncertain failures.

  Each token refresh makes at most one token-endpoint request. This prevents
  google-auth's separate 5xx retries from replaying a potentially rotated
  refresh token. A failed refresh permanently invalidates that credentials
  object. Ambiguous refresh or persistence failures require quarantine.
  """
  try:
    fd = _directory(create=False)
    try:
      with _state_lock(fd):
        client_id, client_secret = _client_values(_read(fd, ENV_FILE))
        channel_id, token = _state_values(_read(fd, STATE_FILE))
    finally:
      os.close(fd)
    from google.auth.transport.requests import Request
    from google.oauth2.credentials import Credentials

    class DurableCredentials(Credentials):
      def refresh(self, request):
        try:
          if getattr(self, "_youtube_refresh_failed", False):
            raise AuthError("Token refresh outcome is uncertain.")
          private_fd = _directory(create=False)
          try:
            with _state_lock(private_fd):
              current_client = _client_values(_read(private_fd, ENV_FILE))
              current_channel, current_token = _state_values(_read(private_fd, STATE_FILE))
              if (current_client != (client_id, client_secret) or current_channel != channel_id
                  or current_token != self.refresh_token):
                raise AuthError("Authorization changed; reload credentials.")

              attempts = 0
              def single_token_request(*args, **kwargs):
                nonlocal attempts
                attempts += 1
                if attempts != 1:
                  raise AuthError("Token refresh outcome is uncertain.")
                response = request(*args, **kwargs)
                if response.status != 200:
                  if response.status == 400:
                    try:
                      body = json.loads(response.data)
                    except (ValueError, TypeError, UnicodeDecodeError):
                      body = None
                    if isinstance(body, dict) and body.get("error") == "invalid_grant":
                      raise AuthError("YouTube authorization required.", kind="auth_required")
                  raise AuthError("Token refresh outcome is uncertain.")
                return response

              with _quiet_google():
                super().refresh(single_token_request)
              new_token = self.refresh_token
              if not _token_valid(new_token):
                raise AuthError("Google returned an invalid refresh token.")
              if new_token != current_token:
                _atomic_write(private_fd, STATE_FILE, _state_text(channel_id, new_token), replace=True)
              if not self.valid:
                raise AuthError("Authorization is invalid or revoked.")
              if self.granted_scopes is not None and not set(SCOPES).issubset(self.granted_scopes):
                raise AuthError("Authorization is missing a required scope.", kind="auth_required")
          finally:
            os.close(private_fd)
        except Exception as error:
          self.token = None
          self._youtube_refresh_failed = True
          kind = error.kind if isinstance(error, AuthError) else "uncertain"
          raise AuthError("Cannot refresh private YouTube authorization.", kind=kind) from None

    credentials = DurableCredentials(
      token=None, refresh_token=token, token_uri=TOKEN_URI,
      client_id=client_id, client_secret=client_secret, scopes=SCOPES)
    credentials.refresh(Request())
    return credentials, channel_id
  except Exception as error:
    kind = error.kind if isinstance(error, AuthError) else "uncertain"
    raise AuthError("Cannot load valid private YouTube authorization.", kind=kind) from None


class _Parser(argparse.ArgumentParser):
  def error(self, message: str) -> None:
    # argparse normally repeats invalid arguments, which could contain secrets.
    self.exit(2, "Invalid command; use configure or authorize.\n")


def main() -> int:
  parser = _Parser(description=__doc__)
  parser.add_argument("command", choices=("configure", "authorize"))
  args = parser.parse_args()
  try:
    if args.command == "configure":
      configure()
    else:
      authorize()
    print("YouTube setup step complete.")
    return 0
  except Exception:
    # Library and OS exceptions can contain credentials or OAuth URLs.
    print("YouTube setup failed; check private storage, consent, and channel selection.",
          file=sys.stderr)
    return 1


if __name__ == "__main__":
  raise SystemExit(main())
