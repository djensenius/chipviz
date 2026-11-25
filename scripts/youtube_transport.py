"""Small, single-request YouTube resumable-upload transport.

create_session(session, metadata, total, content_type="video/mp4") returns a
secret URI. The caller MUST durably persist it before calling either operation
below; this module never creates a replacement session or retries requests.
Metadata is a JSON object with snippet and optional status; the caller is
responsible for excluding local paths and secrets.

make_session(credentials) constructs the supported Google AuthorizedSession
with upload and credential-refresh adapter retries and 401 replays disabled.
The supported youtube_auth credentials also prevent google-auth's separate
token-endpoint 5xx retries; ambiguous refresh/rotation failures are uncertain.
Requests sessions not satisfying this contract are rejected. Test doubles must
set __youtube_transport_test_double__ = True and perform one request without
redirects or retries. They are for offline tests only. Do not mutate the session,
install hooks, use proxies, or enable background credential refresh.

upload_chunk(session, uri, total, offset, data, content_type="video/mp4") sends
one bytes object. Non-final chunks must be multiples of 256 KiB; chunks are
limited to 8 MiB. query_status(session, uri, total) probes the same session.
Both return Progress(offset, video_id); video_id is None until completion.
After an uncertain result, query the persisted session before sending more.
An offset equal to total without a video_id is not completion; query status.
An expired/lost session or unresolved completion requires quarantine, not a
new upload. TransportError.kind is "uncertain", "auth_required", or "quota".
No exception includes a URI, server body, or underlying exception message.
Sanitized public errors discard internal tracebacks and exception chains.

During each request, Python logging is process-wide disabled and stdout/stderr
are redirected, including HTTP wire-debug output. This affects other threads.
Run in an isolated process, or serialize all network operations and logging
configuration, including youtube_auth operations; external tracing, custom
adapters/credentials, proxies, native output, and traceback-local capture are
not supported. Never log the returned URI or inspect/log library exceptions.
"""

import contextlib
import functools
import io
import json
import logging
import re
import sys
import threading
from typing import NamedTuple, Optional
from urllib.parse import parse_qsl, quote, urlencode, urlsplit

# Exception sanitization relies on Python 3.11+ re-raise traceback semantics.
if sys.version_info < (3, 11):
  raise ImportError("YouTube tooling requires Python 3.11 or newer.")

if __package__:
  from .youtube_auth import AuthError
else:
  from youtube_auth import AuthError


_ENDPOINT = "https://www.googleapis.com/upload/youtube/v3/videos"
_PATH = "/upload/youtube/v3/videos"
_TIMEOUT = (10, 60)
_CHUNK_UNIT = 256 * 1024
_MAX_CHUNK = 8 * 1024 * 1024
_ID = re.compile(r"[A-Za-z0-9_-]{11}\Z")
_RANGE = re.compile(r"bytes=0-([0-9]{1,20})\Z")
_QUIET_LOCK = threading.RLock()


class TransportError(Exception):
  """Sanitized failure; an uncertain result must not trigger a new upload."""

  def __init__(self, kind, message):
    super().__init__(message)
    self.kind = kind


def _sanitize_transport_errors(function):
  """Remove internal tracebacks and chains, including an active caller exception."""
  @functools.wraps(function)
  def wrapped(*args, **kwargs):
    try:
      return function(*args, **kwargs)
    except (TransportError, ValueError) as error:
      error.__context__ = None
      error.__cause__ = None
      error.__traceback__ = None
      raise
  return wrapped


class Progress(NamedTuple):
  offset: int
  video_id: Optional[str]


@_sanitize_transport_errors
def make_session(credentials):
  """Construct a no-replay AuthorizedSession for use by these operations."""
  session = None
  try:
    from google.auth.transport.requests import AuthorizedSession
    from requests.adapters import HTTPAdapter

    session = AuthorizedSession(credentials, max_refresh_attempts=0)
    session.trust_env = False
    for scheme in ("http://", "https://"):
      session.mount(scheme, HTTPAdapter(max_retries=0))
      session._auth_request_session.mount(scheme, HTTPAdapter(max_retries=0))
    session._auth_request_session.trust_env = False
    return session
  except Exception:
    if session is not None:
      try:
        session.close()
      except Exception:
        pass
  raise TransportError("uncertain", "Cannot create upload session")


@contextlib.contextmanager
def _quiet_request():
  # Handlers can retain original streams; wire-debug output uses print instead.
  with _QUIET_LOCK:
    previous = logging.root.manager.disable
    logging.disable(max(previous, sys.maxsize))
    try:
      with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
        yield
    finally:
      logging.disable(previous)


def _check_session(session, uri):
  # Explicit offline test doubles are responsible for honoring the contract.
  def test_double():
    if getattr(session, "__youtube_transport_test_double__", False) is not True:
      raise TransportError("uncertain", "Unsafe upload session configuration")
    return False

  try:
    from requests import Session
    from requests.adapters import HTTPAdapter
  except ImportError:
    return test_double()
  if not isinstance(session, Session):
    return test_double()
  try:
    from google.auth.transport.requests import AuthorizedSession
  except ImportError:
    raise TransportError("uncertain", "Unsafe upload session configuration") from None
  try:
    auth_session = session._auth_request_session
    adapters = (session.get_adapter(uri),
                auth_session.get_adapter("https://oauth2.googleapis.com/"))
    valid = (
      isinstance(session, AuthorizedSession)
      and session._max_refresh_attempts == 0
      and session._auth_request.session is auth_session
      and not session.trust_env and not auth_session.trust_env
      and not session.proxies and not auth_session.proxies
      and not session.params and not session.auth and session.verify is not False
      and not auth_session.params and not auth_session.auth and auth_session.verify is not False
      and not any(session.hooks.values()) and not any(auth_session.hooks.values())
      and not getattr(session.credentials, "_use_non_blocking_refresh", False)
      and all(isinstance(adapter, HTTPAdapter) and adapter.max_retries.total == 0
              for adapter in adapters)
    )
  except (AttributeError, TypeError, ValueError):
    valid = False
  if not valid:
    raise TransportError("uncertain", "Unsafe upload session configuration")
  return True


def _prepared_uri(method, uri, params):
  # Use the actual Requests preparation rules when Requests is installed.
  try:
    from requests import Request
  except ImportError:
    if params:
      uri += ("&" if "?" in uri else "?") + urlencode(params)
    unreserved = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"
    def unquote_unreserved(match):
      char = chr(int(match.group()[1:], 16))
      return char if char in unreserved else match.group()
    return quote(re.sub(r"%[0-9a-fA-F]{2}", unquote_unreserved, uri),
                 safe="!#$%&'()*+,/:;=?@[]~")
  return Request(method, uri, params=params).prepare().url


def _uri(value, *, session_uri):
  if (not isinstance(value, str) or not value.isascii() or len(value) > 8192
      or any(ord(char) <= 32 or ord(char) == 127 for char in value)
      or "#" in value or "\\" in value
      or re.search(r"%(?![0-9a-fA-F]{2})", value)):
    raise TransportError("uncertain", "Invalid upload URI")
  try:
    parts = urlsplit(value)
    if (parts.scheme != "https" or parts.netloc != "www.googleapis.com"
        or parts.path != _PATH or parts.fragment):
      raise ValueError
    if session_uri:
      pairs = parse_qsl(parts.query, keep_blank_values=True, strict_parsing=True)
      ids = [v for k, v in pairs if k == "upload_id"]
      if (len(ids) != 1 or not ids[0]
          or any(any(ord(c) < 32 or ord(c) == 127 for c in k + v)
                 for k, v in pairs)):
        raise ValueError
  except ValueError:
    raise TransportError("uncertain", "Invalid upload URI") from None


def _total(total):
  if type(total) is not int or total <= 0:
    raise ValueError("total must be a positive integer")


def _content_type(content_type):
  if (not isinstance(content_type, str)
      or not re.fullmatch(r"[A-Za-z0-9!#$&^_.+-]+/[A-Za-z0-9!#$&^_.+-]+",
                          content_type)):
    raise ValueError("invalid content type")


def _request(session, method, uri, **kwargs):
  real_session = _check_session(session, uri)
  try:
    expected = _prepared_uri(method.upper(), uri, kwargs.get("params"))
  except Exception:
    raise TransportError("uncertain", "Invalid upload URI") from None
  failure = None
  try:
    with _quiet_request():
      response = getattr(session, method)(
        uri, timeout=_TIMEOUT, allow_redirects=False, **kwargs
      )
  except Exception as error:
    failure = (error.kind if isinstance(error, AuthError)
               and error.kind == "auth_required" else "uncertain")
  if failure:
    message = ("YouTube authorization required" if failure == "auth_required"
               else "Upload request failed; check session status")
    raise TransportError(failure, message)
  if getattr(response, "history", None):
    raise TransportError("uncertain", "Unexpected redirect")
  response_url = getattr(response, "url", None)
  if real_session and response_url is None:
    raise TransportError("uncertain", "Missing response URI")
  if response_url is not None:
    _uri(response_url, session_uri=method == "put")
    if response_url != expected:
      raise TransportError("uncertain", "Unexpected response URI")
  return response


def _failure(response):
  code = response.status_code
  if code == 401:
    raise TransportError("auth_required", "YouTube authorization required")
  if code in (400, 403):
    try:
      body = response.json()
      errors = body.get("error", {}).get("errors", [])
      reasons = {item.get("reason") for item in errors if isinstance(item, dict)}
    except Exception:
      reasons = set()
    if reasons & {"quotaExceeded", "dailyLimitExceeded", "uploadLimitExceeded"}:
      raise TransportError("quota", "YouTube upload quota exceeded") from None
    if code == 403 and reasons & {"authError", "required", "insufficientPermissions"}:
      raise TransportError("auth_required", "YouTube authorization required") from None
  raise TransportError("uncertain", "Upload response requires quarantine or status check")


@_sanitize_transport_errors
def create_session(session, metadata, total, content_type="video/mp4"):
  """Initiate an unlisted upload; return the secret URI without sending media."""
  _total(total)
  _content_type(content_type)
  if not isinstance(metadata, dict) or not isinstance(metadata.get("snippet"), dict):
    raise ValueError("metadata must contain a snippet object")
  try:
    copied = json.loads(json.dumps(metadata, allow_nan=False))
  except (TypeError, ValueError):
    raise ValueError("metadata must be JSON serializable") from None
  if set(copied) - {"snippet", "status"} or not isinstance(copied.get("status", {}), dict):
    raise ValueError("metadata supports only snippet and status objects")
  status = copied.setdefault("status", {})
  if status.get("privacyStatus", "unlisted") != "unlisted":
    raise ValueError("privacyStatus must be unlisted")
  status["privacyStatus"] = "unlisted"
  response = _request(
    session, "post", _ENDPOINT,
    params={"uploadType": "resumable", "part": "snippet,status",
            "notifySubscribers": "false"},
    headers={"X-Upload-Content-Length": str(total),
             "X-Upload-Content-Type": content_type},
    json=copied,
  )
  if response.status_code not in (200, 201):
    _failure(response)
  uri = response.headers.get("Location")
  _uri(uri, session_uri=True)
  return uri


def _progress(response, total, minimum, maximum, final_allowed):
  if response.headers.get("Location") is not None:
    raise TransportError("uncertain", "Unexpected redirect")
  if response.status_code == 308:
    value = response.headers.get("Range")
    if value is None:
      offset = 0
    else:
      match = _RANGE.fullmatch(value) if isinstance(value, str) else None
      if not match:
        raise TransportError("uncertain", "Invalid upload range")
      offset = int(match.group(1)) + 1
    if not minimum <= offset <= maximum or offset > total:
      raise TransportError("uncertain", "Invalid upload range")
    return Progress(offset, None)
  if response.status_code in (200, 201):
    if not final_allowed:
      raise TransportError("uncertain", "Unexpected upload completion")
    try:
      video_id = response.json()["id"]
    except Exception:
      video_id = None
    if not isinstance(video_id, str) or not _ID.fullmatch(video_id):
      raise TransportError("uncertain", "Invalid upload completion")
    return Progress(total, video_id)
  _failure(response)


@_sanitize_transport_errors
def upload_chunk(session, uri, total, offset, data, content_type="video/mp4"):
  """Send one bounded chunk at offset on a previously persisted session."""
  _uri(uri, session_uri=True)
  _total(total)
  _content_type(content_type)
  if type(offset) is not int or offset < 0 or offset >= total:
    raise ValueError("invalid offset")
  if not isinstance(data, bytes) or not data or len(data) > _MAX_CHUNK:
    raise ValueError("data must be a nonempty bounded bytes object")
  end = offset + len(data)
  if end > total or (end < total and len(data) % _CHUNK_UNIT):
    raise ValueError("invalid chunk length")
  response = _request(
    session, "put", uri, data=data,
    headers={"Content-Type": content_type, "Content-Length": str(len(data)),
             "Content-Range": f"bytes {offset}-{end - 1}/{total}"},
  )
  return _progress(response, total, offset, end, end == total)


@_sanitize_transport_errors
def query_status(session, uri, total):
  """Probe the persisted session; a completed result includes its video ID."""
  _uri(uri, session_uri=True)
  _total(total)
  response = _request(
    session, "put", uri, data=b"",
    headers={"Content-Length": "0", "Content-Range": f"bytes */{total}"},
  )
  return _progress(response, total, 0, total, True)
