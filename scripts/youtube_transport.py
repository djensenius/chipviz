"""Small, single-request YouTube resumable-upload transport.

create_session(session, metadata, total, content_type="video/mp4") returns a
secret URI. The caller MUST durably persist it before calling either operation
below; this module never creates a replacement session or retries requests.
Metadata is a JSON object with snippet and optional status; the caller is
responsible for excluding local paths and secrets.

upload_chunk(session, uri, total, offset, data, content_type="video/mp4") sends
one bytes object. Non-final chunks must be multiples of 256 KiB; chunks are
limited to 8 MiB. query_status(session, uri, total) probes the same session.
Both return Progress(offset, video_id); video_id is None until completion.
After an uncertain result, query the persisted session before sending more.
An offset equal to total without a video_id is not completion; query status.
An expired/lost session or unresolved completion requires quarantine, not a
new upload. TransportError.kind is "uncertain", "auth_required", or "quota".
No exception includes a URI, server body, or underlying exception message.
"""

import json
import re
from typing import NamedTuple
from urllib.parse import parse_qsl, urlsplit


_ENDPOINT = "https://www.googleapis.com/upload/youtube/v3/videos"
_PATH = "/upload/youtube/v3/videos"
_TIMEOUT = (10, 60)
_CHUNK_UNIT = 256 * 1024
_MAX_CHUNK = 8 * 1024 * 1024
_ID = re.compile(r"[A-Za-z0-9_-]{11}\Z")
_RANGE = re.compile(r"bytes=0-([0-9]{1,20})\Z")


class TransportError(Exception):
  """Sanitized failure; an uncertain result must not trigger a new upload."""

  def __init__(self, kind, message):
    super().__init__(message)
    self.kind = kind


class Progress(NamedTuple):
  offset: int
  video_id: str | None


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
  try:
    response = getattr(session, method)(
      uri, timeout=_TIMEOUT, allow_redirects=False, **kwargs
    )
  except Exception:
    raise TransportError("uncertain", "Upload request failed; check session status") from None
  if getattr(response, "history", None):
    raise TransportError("uncertain", "Unexpected redirect")
  response_url = getattr(response, "url", None)
  if response_url is not None:
    _uri(response_url, session_uri=method == "put")
    if method == "put" and response_url != uri:
      raise TransportError("uncertain", "Unexpected response URI")
  return response


def _failure(response):
  code = response.status_code
  if code == 401:
    raise TransportError("auth_required", "YouTube authorization required")
  if code == 403:
    try:
      body = response.json()
      errors = body.get("error", {}).get("errors", [])
      reasons = {item.get("reason") for item in errors if isinstance(item, dict)}
    except (ValueError, TypeError, AttributeError):
      reasons = set()
    if reasons & {"quotaExceeded", "dailyLimitExceeded", "uploadLimitExceeded"}:
      raise TransportError("quota", "YouTube upload quota exceeded") from None
    if reasons & {"authError", "required", "insufficientPermissions"}:
      raise TransportError("auth_required", "YouTube authorization required") from None
  raise TransportError("uncertain", "Upload response requires quarantine or status check")


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
    except (ValueError, TypeError, KeyError):
      raise TransportError("uncertain", "Invalid upload completion") from None
    if not isinstance(video_id, str) or not _ID.fullmatch(video_id):
      raise TransportError("uncertain", "Invalid upload completion")
    return Progress(total, video_id)
  _failure(response)


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


def query_status(session, uri, total):
  """Probe the persisted session; a completed result includes its video ID."""
  _uri(uri, session_uri=True)
  _total(total)
  response = _request(
    session, "put", uri, data=b"",
    headers={"Content-Length": "0", "Content-Range": f"bytes */{total}"},
  )
  return _progress(response, total, 0, total, True)
