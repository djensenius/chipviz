import contextlib
import datetime
import io
import json
import logging
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

try:
  import google.auth.credentials
  import google.auth.exceptions
  from google.oauth2 import reauth as google_reauth
  from google.auth.transport.requests import AuthorizedSession
  import requests
  from requests.adapters import HTTPAdapter
except ImportError:
  google = None
  requests = None
  AuthorizedSession = None
  HTTPAdapter = object
  google_reauth = None

from scripts import youtube_auth
from scripts import youtube_transport as transport


URI = "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=secret-session"
OTHER_URI = "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=other"
UNIT = 256 * 1024
VIDEO_ID = "abcdefgh_12"


class Response:
  def __init__(self, status, headers=None, body=None, url=None, history=None):
    self.status_code = status
    self.headers = headers or {}
    self.body = body
    self.url = url
    self.history = history or []

  def json(self):
    if isinstance(self.body, Exception):
      raise self.body
    return self.body


class Session:
  __youtube_transport_test_double__ = True

  def __init__(self, *responses):
    self.responses = list(responses)
    self.calls = []

  def request(self, method, uri, **kwargs):
    self.calls.append((method, uri, kwargs))
    result = self.responses.pop(0)
    if isinstance(result, Exception):
      raise result
    return result

  def post(self, uri, **kwargs):
    return self.request("post", uri, **kwargs)

  def put(self, uri, **kwargs):
    return self.request("put", uri, **kwargs)


class YouTubeTransportTests(unittest.TestCase):
  def assert_kind(self, kind, function, *args):
    with self.assertRaises(transport.TransportError) as caught:
      function(*args)
    self.assertEqual(caught.exception.kind, kind)
    self.assertNotIn("secret-session", str(caught.exception))
    return caught.exception

  def test_initiation_is_separate_and_explicitly_unlisted(self):
    metadata = {"snippet": {"title": "Synthetic"}, "status": {"madeForKids": False}}
    session = Session(Response(200, {"Location": URI}))
    self.assertEqual(transport.create_session(session, metadata, UNIT + 1), URI)
    self.assertEqual(len(session.calls), 1)
    method, url, kwargs = session.calls[0]
    self.assertEqual(method, "post")
    self.assertEqual(url, "https://www.googleapis.com/upload/youtube/v3/videos")
    self.assertEqual(kwargs["params"], {
      "uploadType": "resumable", "part": "snippet,status", "notifySubscribers": "false",
    })
    self.assertEqual(kwargs["headers"], {
      "X-Upload-Content-Length": str(UNIT + 1), "X-Upload-Content-Type": "video/mp4",
    })
    self.assertEqual(kwargs["json"]["status"], {
      "madeForKids": False, "privacyStatus": "unlisted",
    })
    self.assertNotIn("privacyStatus", metadata["status"])
    self.assertNotIn("data", kwargs)
    self.assertFalse(kwargs["allow_redirects"])
    self.assertEqual(kwargs["timeout"], (10, 60))

  def test_chunks_and_exact_ranges(self):
    session = Session(
      Response(308, {"Range": f"bytes=0-{UNIT - 1}"}, url=URI),
      Response(308, {"Range": f"bytes=0-{2 * UNIT - 1}"}, url=URI),
      Response(201, body={"id": VIDEO_ID}, url=URI),
    )
    total = 2 * UNIT + 3
    for offset in (0, UNIT):
      result = transport.upload_chunk(session, URI, total, offset, b"x" * UNIT)
      self.assertEqual(result, (offset + UNIT, None))
    self.assertEqual(transport.upload_chunk(session, URI, total, 2 * UNIT, b"end"),
                     (total, VIDEO_ID))
    for index, (_, url, kwargs) in enumerate(session.calls):
      data = kwargs["data"]
      offset = index * UNIT
      self.assertEqual(url, URI)
      self.assertEqual(kwargs["headers"], {
        "Content-Type": "video/mp4", "Content-Length": str(len(data)),
        "Content-Range": f"bytes {offset}-{offset + len(data) - 1}/{total}",
      })
      self.assertFalse(kwargs["allow_redirects"])
      self.assertEqual(kwargs["timeout"], (10, 60))

  def test_recover_partial_and_lost_final_response_by_status(self):
    total = UNIT + 3
    session = Session(
      TimeoutError("URI " + URI),
      Response(308, {"Range": f"bytes=0-{UNIT - 1}"}),
      TimeoutError("lost final response " + URI),
      Response(200, body={"id": VIDEO_ID}),
    )
    self.assert_kind("uncertain", transport.upload_chunk, session, URI, total, 0, b"x" * UNIT)
    self.assertEqual(transport.query_status(session, URI, total), (UNIT, None))
    self.assert_kind("uncertain", transport.upload_chunk, session, URI, total, UNIT, b"end")
    self.assertEqual(transport.query_status(session, URI, total), (total, VIDEO_ID))
    for index in (1, 3):
      headers = session.calls[index][2]["headers"]
      self.assertEqual(headers, {"Content-Length": "0", "Content-Range": f"bytes */{total}"})
      self.assertEqual(session.calls[index][2]["data"], b"")
    self.assertEqual([call[0] for call in session.calls], ["put"] * 4)

  def test_missing_range_means_zero_and_partial_ack_is_usable(self):
    session = Session(Response(308), Response(308, {"Range": "bytes=0-9"}),
                      Response(308, {"Range": f"bytes=0-{UNIT - 1}"}))
    self.assertEqual(transport.query_status(session, URI, UNIT), (0, None))
    self.assertEqual(transport.upload_chunk(session, URI, UNIT, 0, b"x" * UNIT), (10, None))
    self.assertEqual(transport.query_status(session, URI, UNIT), (UNIT, None))

  def test_invalid_ranges_and_completion_fail_closed(self):
    for value in ("bytes=1-9", "bytes=0-999", "bytes=0--1", "bytes=0-1,2-3",
                  "bytes 0-9", "", 4):
      with self.subTest(value=value):
        self.assert_kind("uncertain", transport.query_status,
                         Session(Response(308, {"Range": value})), URI, 100)
    self.assert_kind("uncertain", transport.upload_chunk, Session(Response(308)),
                     URI, UNIT + 1, UNIT, b"x")
    self.assert_kind("uncertain", transport.upload_chunk,
                     Session(Response(308, {"Range": f"bytes=0-{UNIT}"})),
                     URI, UNIT + 1, 0, b"x" * UNIT)
    for body in (None, {}, {"id": "too-short"}, {"id": "abcdefgh!12"},
                 {"id": 123}, ValueError(URI), RuntimeError(URI)):
      with self.subTest(body=body):
        error = self.assert_kind("uncertain", transport.query_status,
                                 Session(Response(200, body=body)), URI, 10)
        self.assertIsNone(error.__context__)
    self.assert_kind("uncertain", transport.upload_chunk,
                     Session(Response(200, body={"id": VIDEO_ID})), URI, UNIT + 1, 0,
                     b"x" * UNIT)

  def test_invalid_uris_and_redirects_are_never_followed(self):
    invalid = (
      "http://www.googleapis.com/upload/youtube/v3/videos?upload_id=x",
      "https://evil.example/upload/youtube/v3/videos?upload_id=x",
      "https://www.googleapis.com.evil.example/upload/youtube/v3/videos?upload_id=x",
      "https://user@www.googleapis.com/upload/youtube/v3/videos?upload_id=x",
      "https://www.googleapis.com:443/upload/youtube/v3/videos?upload_id=x",
      "https://www.googleapis.com/upload/youtube/v3/other?upload_id=x",
      URI + "#fragment", URI + "%0a", URI + "&upload_id=another",
      "https://www.googleapis.com/upload/youtube/v3/videos?foo=bar",
    )
    for uri in invalid:
      with self.subTest(uri=uri):
        session = Session()
        self.assert_kind("uncertain", transport.query_status, session, uri, 10)
        self.assertEqual(session.calls, [])
        self.assert_kind("uncertain", transport.create_session,
                         Session(Response(200, {"Location": uri})),
                         {"snippet": {}}, 10)
    for response in (
      Response(302, {"Location": "https://evil.example/"}),
      Response(308, {"Location": OTHER_URI}),
      Response(308, url=OTHER_URI),
      Response(308, url="https://evil.example/"),
      Response(308, history=[Response(302)]),
    ):
      self.assert_kind("uncertain", transport.query_status, Session(response), URI, 10)

  def test_failures_classified_and_never_leak(self):
    for status, body, kind in (
      (401, None, "auth_required"),
      (403, {"error": {"errors": [{"reason": "authError"}]}}, "auth_required"),
      (403, {"error": {"errors": [{"reason": "quotaExceeded"}]}}, "quota"),
      (403, {"error": {"errors": [{"reason": "dailyLimitExceeded"}]}}, "quota"),
      (403, {"error": {"errors": [{"reason": "uploadLimitExceeded"}]}}, "quota"),
      (400, {"error": {"errors": [{"reason": "uploadLimitExceeded"}]}}, "quota"),
      (400, {"error": {"message": URI}}, "uncertain"),
      (403, {"error": {"message": URI}}, "uncertain"),
      (404, {"secret": URI}, "uncertain"),
      (410, {"secret": URI}, "uncertain"),
      (500, {"secret": URI}, "uncertain"),
    ):
      with self.subTest(status=status, body=body):
        session = Session(Response(status, body=body))
        with contextlib.redirect_stdout(io.StringIO()) as stdout:
          with contextlib.redirect_stderr(io.StringIO()) as stderr:
            error = self.assert_kind(kind, transport.query_status, session, URI, 10)
        self.assertNotIn(URI, repr(error))
        self.assertEqual(stdout.getvalue(), "")
        self.assertEqual(stderr.getvalue(), "")
        self.assertEqual(len(session.calls), 1)

  def test_validation_prevents_requests_and_metadata_mutation(self):
    session = Session()
    invalid_calls = (
      (transport.create_session, (session, {"snippet": {}, "status": {"privacyStatus": "public"}}, 1)),
      (transport.create_session, (session, {"snippet": {}}, 0)),
      (transport.create_session, (session, {"snippet": {}, "other": {}}, 1)),
      (transport.upload_chunk, (session, URI, UNIT + 1, 0, b"x")),
      (transport.upload_chunk, (session, URI, 1, 1, b"x")),
      (transport.upload_chunk, (session, URI, 1, 0, b"xx")),
      (transport.upload_chunk, (session, URI, 10 * UNIT, 0, b"x" * (9 * 1024 * 1024))),
    )
    for function, args in invalid_calls:
      with self.subTest(function=function, args=args[:2]):
        with self.assertRaises(ValueError):
          function(*args)
    self.assertEqual(session.calls, [])

  def test_unmarked_duck_session_is_rejected(self):
    session = mock.Mock()
    self.assert_kind("uncertain", transport.query_status, session, URI, 1)
    session.put.assert_not_called()

  def test_error_traceback_does_not_expose_underlying_exception(self):
    session = Session(RuntimeError("token and " + URI))
    with mock.patch("sys.stderr", new_callable=io.StringIO) as stderr:
      try:
        transport.query_status(session, URI, 1)
      except transport.TransportError:
        import traceback
        traceback.print_exc()
    self.assertNotIn(URI, stderr.getvalue())
    self.assertNotIn("token", stderr.getvalue())

  def test_sanitized_errors_clear_even_an_active_caller_exception(self):
    try:
      raise RuntimeError("synthetic-outer-secret")
    except RuntimeError:
      with self.assertRaises(transport.TransportError) as error:
        transport.query_status(Session(), URI + "%", 1)
      self.assertIsNone(error.exception.__context__)
      self.assertIsNone(error.exception.__cause__)
      with self.assertRaises(ValueError) as invalid:
        transport.upload_chunk(Session(), URI, 1, 0, b"")
      self.assertIsNone(invalid.exception.__context__)
      self.assertIsNone(invalid.exception.__cause__)

  def test_transport_failure_drops_internal_traceback(self):
    try:
      transport.query_status(Session(RuntimeError("secret " + URI)), URI, 1)
    except transport.TransportError as error:
      self.assertIsNone(error.__context__)
      self.assertIsNone(error.__cause__)
      frame = error.__traceback__
      while frame:
        self.assertNotEqual(frame.tb_frame.f_code.co_filename, transport.__file__)
        frame = frame.tb_next
    else:
      self.fail("expected sanitized transport error")

  def test_auth_failure_kind_is_preserved_without_leaking(self):
    for kind in ("auth_required", "uncertain"):
      with self.subTest(kind=kind):
        error = youtube_auth.AuthError("secret " + URI, kind=kind)
        result = self.assert_kind(kind, transport.query_status, Session(error), URI, 1)
        self.assertIsNone(result.__context__)

  def test_initiation_with_uncertain_auth_failure_is_not_replayed(self):
    session = Session(youtube_auth.AuthError("secret " + URI))
    error = self.assert_kind("uncertain", transport.create_session,
                             session, {"snippet": {}}, 1)
    self.assertIsNone(error.__context__)
    self.assertEqual(len(session.calls), 1)
    self.assertEqual(session.calls[0][0], "post")


class OfflineCredentials(google.auth.credentials.Credentials if google else object):
  def __init__(self, refresh_error=None):
    super().__init__()
    self.token = "synthetic-token" if refresh_error is None else None
    self.refresh_error = refresh_error
    self.refreshes = 0

  def refresh(self, request):
    self.refreshes += 1
    if self.refresh_error:
      raise self.refresh_error
    self.token = "synthetic-token"


class OfflineAdapter(HTTPAdapter):
  def __init__(self, status=308, error=None, response_url=None, body=b"{}", location=None):
    super().__init__(max_retries=0)
    self.status = status
    self.error = error
    self.response_url = response_url
    self.body = body
    self.location = location
    self.calls = []

  def send(self, request, **kwargs):
    self.calls.append(request)
    logging.getLogger("urllib3.connectionpool").debug(
      'PUT %s Authorization %s body %s',
      request.url, request.headers.get("Authorization"), request.body)
    print("wire debug " + request.url + " " + request.headers.get("Authorization", ""))
    print("wire error " + request.url, file=sys.stderr)
    if self.error:
      raise self.error
    response = requests.Response()
    response.status_code = self.status
    response.url = self.response_url or request.url
    response.request = request
    response._content = self.body
    if self.location:
      response.headers["Location"] = self.location
    return response


@unittest.skipUnless(google is not None and requests is not None,
                     "optional Google and Requests dependencies not installed")
class RealLibraryOfflineTests(unittest.TestCase):
  def setUp(self):
    self.credentials = OfflineCredentials()
    self.session = transport.make_session(self.credentials)
    self.addCleanup(self.session.close)
    self.adapter = OfflineAdapter()
    self.session.mount("https://", self.adapter)

  def test_factory_disables_replays_and_401_is_sent_once(self):
    self.assertEqual(self.session._max_refresh_attempts, 0)
    self.assertEqual(self.session.get_adapter(URI).max_retries.total, 0)
    self.assertEqual(
      self.session._auth_request_session.get_adapter("https://oauth2.googleapis.com/").max_retries.total,
      0,
    )
    self.adapter.status = 401
    with self.assertRaises(transport.TransportError) as caught:
      transport.upload_chunk(self.session, URI, 1, 0, b"x")
    self.assertEqual(caught.exception.kind, "auth_required")
    self.assertEqual(len(self.adapter.calls), 1)
    self.assertEqual(self.credentials.refreshes, 0)

  def test_session_factory_failure_is_sanitized(self):
    with mock.patch("google.auth.transport.requests.AuthorizedSession",
                    side_effect=RuntimeError("synthetic-secret " + URI)):
      with self.assertRaises(transport.TransportError) as error:
        transport.make_session(self.credentials)
    self.assertEqual(error.exception.kind, "uncertain")
    self.assertNotIn("synthetic-secret", repr(error.exception))
    self.assertNotIn(URI, repr(error.exception))
    self.assertIsNone(error.exception.__context__)
    self.assertIsNone(error.exception.__cause__)

  def test_logging_wire_debug_and_exceptions_never_expose_secrets(self):
    logged = io.StringIO()
    handler = logging.StreamHandler(logged)
    logger = logging.getLogger("urllib3.connectionpool")
    old_level = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(handler)
    self.addCleanup(logger.removeHandler, handler)
    self.addCleanup(logger.setLevel, old_level)
    original_disable = logging.root.manager.disable
    output, errors = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
      self.assertEqual(transport.query_status(self.session, URI, 1), (0, None))
      self.adapter.error = requests.exceptions.ConnectionError("failed " + URI + " synthetic-token")
      with self.assertRaises(transport.TransportError) as caught:
        transport.query_status(self.session, URI, 1)
      import traceback
      traceback.print_exception(caught.exception)
    self.assertEqual(caught.exception.kind, "uncertain")
    self.assertIsNone(caught.exception.__context__)
    for text in (logged.getvalue(), output.getvalue(), errors.getvalue()):
      self.assertNotIn("secret-session", text)
      self.assertNotIn("synthetic-token", text)
    self.assertEqual(logging.root.manager.disable, original_disable)

  def test_auth_quiet_context_preserves_stronger_transport_suppression(self):
    output = io.StringIO()
    handler = logging.StreamHandler(output)
    logger = logging.getLogger("youtube-transport-nested-test")
    logger.addHandler(handler)
    self.addCleanup(logger.removeHandler, handler)
    with transport._quiet_request():
      with youtube_auth._quiet_google():
        logger.log(logging.CRITICAL + 1, "secret " + URI)
    self.assertEqual(output.getvalue(), "")

  def test_auth_refresh_failures_are_sanitized_without_upload(self):
    cases = (
      (youtube_auth.AuthError("secret " + URI, kind="auth_required"), "auth_required"),
      (youtube_auth.AuthError("secret " + URI), "uncertain"),
      (google.auth.exceptions.RefreshError("secret " + URI, retryable=True), "uncertain"),
      (google.auth.exceptions.RefreshError("secret " + URI), "uncertain"),
      (google.auth.exceptions.TransportError("secret " + URI), "uncertain"),
    )
    for error, kind in cases:
      with self.subTest(error=type(error), kind=kind):
        credentials = OfflineCredentials(error)
        session = transport.make_session(credentials)
        self.addCleanup(session.close)
        adapter = OfflineAdapter()
        session.mount("https://", adapter)
        with self.assertRaises(transport.TransportError) as caught:
          transport.query_status(session, URI, 1)
        self.assertEqual(caught.exception.kind, kind)
        self.assertNotIn("secret-session", str(caught.exception))
        self.assertIsNone(caught.exception.__context__)
        self.assertEqual(credentials.refreshes, 1)
        self.assertEqual(adapter.calls, [])

  def test_rotation_persistence_failure_prevents_upload_and_replay(self):
    with tempfile.TemporaryDirectory() as temporary:
      home = Path(temporary).resolve() / "home"
      home.mkdir(mode=0o700)
      with mock.patch.object(youtube_auth.Path, "home", return_value=home):
        with mock.patch.object(youtube_auth.getpass, "getpass", side_effect=(
            "synthetic.apps.googleusercontent.com", "synthetic-secret")):
          youtube_auth.configure()
        state = home / ".config" / "chipviz" / youtube_auth.STATE_FILE
        state.write_text(youtube_auth._state_text("UC" + "a" * 22, "synthetic-refresh"))
        state.chmod(0o600)
        expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
        expiry += datetime.timedelta(hours=1)
        initial = ("synthetic-access", "synthetic-refresh", expiry, {}, None)
        with mock.patch.object(google_reauth, "refresh_grant", return_value=initial):
          credentials, _ = youtube_auth.load_credentials()
        credentials.token = None
        session = transport.make_session(credentials)
        self.addCleanup(session.close)
        upload = OfflineAdapter()
        session.mount("https://", upload)
        token = OfflineAdapter(status=200, body=json.dumps({
          "access_token": "synthetic-access-two", "expires_in": 3600,
          "refresh_token": "synthetic-rotated",
        }).encode())
        def send_token(_adapter, request, **kwargs):
          return token.send(request, **kwargs)
        with mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send_token), \
            mock.patch.object(youtube_auth, "_atomic_write", side_effect=OSError("synthetic-secret")):
          for _ in range(2):
            with self.assertRaises(transport.TransportError) as error:
              transport.create_session(session, {"snippet": {}}, 1)
            self.assertEqual(error.exception.kind, "uncertain")
            self.assertNotIn("synthetic-secret", str(error.exception))
            self.assertIsNone(error.exception.__context__)
        self.assertEqual(len(token.calls), 1)
        self.assertEqual(upload.calls, [])

  def test_authorized_session_refresh_blocks_all_token_redirects(self):
    output = io.StringIO()
    handler = logging.StreamHandler(output)
    logger = logging.getLogger("urllib3.connectionpool")
    previous = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(handler)
    self.addCleanup(logger.removeHandler, handler)
    self.addCleanup(logger.setLevel, previous)
    with tempfile.TemporaryDirectory() as temporary:
      home = Path(temporary).resolve() / "home"
      home.mkdir(mode=0o700)
      with mock.patch.object(youtube_auth.Path, "home", return_value=home):
        with mock.patch.object(youtube_auth.getpass, "getpass", side_effect=(
            "synthetic.apps.googleusercontent.com", "synthetic-secret")):
          youtube_auth.configure()
        state = home / ".config" / "chipviz" / youtube_auth.STATE_FILE
        state.write_text(youtube_auth._state_text("UC" + "a" * 22, "synthetic-refresh"))
        state.chmod(0o600)
        expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
        expiry += datetime.timedelta(hours=1)
        initial = ("synthetic-access", "synthetic-refresh", expiry, {}, None)
        for status in (301, 302, 303, 307, 308):
          with self.subTest(status=status):
            with mock.patch.object(google_reauth, "refresh_grant", return_value=initial):
              credentials, _ = youtube_auth.load_credentials()
            credentials.token = None
            session = transport.make_session(credentials)
            self.addCleanup(session.close)
            upload = OfflineAdapter()
            session.mount("https://", upload)
            token = OfflineAdapter(status=status, location="https://attacker.invalid/steal")
            def send_token(_adapter, request, **kwargs):
              return token.send(request, **kwargs)
            with mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send_token), \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
              with self.assertRaises(transport.TransportError) as error:
                transport.create_session(session, {"snippet": {}}, 1)
            self.assertEqual(error.exception.kind, "uncertain")
            self.assertNotIn("synthetic-refresh", str(error.exception))
            self.assertNotIn("attacker.invalid", str(error.exception))
            self.assertIsNone(error.exception.__context__)
            self.assertIsNone(error.exception.__cause__)
            self.assertEqual(len(token.calls), 1)
            self.assertEqual(token.calls[0].url, youtube_auth.TOKEN_URI)
            self.assertEqual(upload.calls, [])
    for secret in ("synthetic-secret", "synthetic-refresh", "attacker.invalid",
                   youtube_auth.TOKEN_URI):
      self.assertNotIn(secret, output.getvalue())

  def test_malformed_refresh_cannot_expose_secrets_through_transport_error(self):
    with tempfile.TemporaryDirectory() as temporary:
      home = Path(temporary).resolve() / "home"
      home.mkdir(mode=0o700)
      with mock.patch.object(youtube_auth.Path, "home", return_value=home):
        with mock.patch.object(youtube_auth.getpass, "getpass", side_effect=(
            "synthetic.apps.googleusercontent.com", "synthetic-secret")):
          youtube_auth.configure()
        state = home / ".config" / "chipviz" / youtube_auth.STATE_FILE
        state.write_text(youtube_auth._state_text("UC" + "a" * 22, "synthetic-refresh"))
        state.chmod(0o600)
        expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None)
        expiry += datetime.timedelta(hours=1)
        initial = ("synthetic-access", "synthetic-refresh", expiry, {}, None)
        with mock.patch.object(google_reauth, "refresh_grant", return_value=initial):
          credentials, _ = youtube_auth.load_credentials()
        credentials.token = None
        session = transport.make_session(credentials)
        self.addCleanup(session.close)
        upload = OfflineAdapter()
        session.mount("https://", upload)
        token = OfflineAdapter(status=200, body=json.dumps({
          "refresh_token": "synthetic-rotated-token",
          "client_secret": "synthetic-secret",
        }).encode())
        def send_token(_adapter, request, **kwargs):
          return token.send(request, **kwargs)
        with mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send_token):
          try:
            raise RuntimeError("synthetic-outer-secret")
          except RuntimeError:
            try:
              transport.create_session(session, {"snippet": {}}, 1)
            except transport.TransportError as error:
              self.assertEqual(error.kind, "uncertain")
              self.assertIsNone(error.__context__)
              self.assertIsNone(error.__cause__)
              for secret in ("synthetic-rotated-token", "synthetic-secret", "synthetic-refresh",
                             youtube_auth.TOKEN_URI, "synthetic-outer-secret"):
                self.assertNotIn(secret, repr(error))
                self.assertNotIn(secret, repr(vars(error)))
              frame = error.__traceback__
              while frame:
                self.assertNotEqual(frame.tb_frame.f_code.co_filename, transport.__file__)
                self.assertNotEqual(frame.tb_frame.f_code.co_filename, youtube_auth.__file__)
                self.assertNotIn("/site-packages/", frame.tb_frame.f_code.co_filename)
                frame = frame.tb_next
            else:
              self.fail("expected sanitized transport error")
        self.assertEqual(len(token.calls), 1)
        self.assertEqual(upload.calls, [])

  def test_normalized_response_url_matches_requests_preparation(self):
    uri = URI + "%7E"
    self.assertEqual(transport.query_status(self.session, uri, 1), (0, None))
    expected = requests.Request("PUT", uri).prepare().url
    self.assertTrue(expected.endswith("secret-session~"))
    self.assertEqual(self.adapter.calls[0].url, expected)
    self.adapter.response_url = OTHER_URI
    with self.assertRaises(transport.TransportError) as caught:
      transport.query_status(self.session, uri, 1)
    self.assertEqual(caught.exception.kind, "uncertain")
    self.adapter.response_url = "https://evil.example/upload/youtube/v3/videos?upload_id=bad"
    with self.assertRaises(transport.TransportError):
      transport.query_status(self.session, uri, 1)

  def test_initiation_response_must_match_prepared_request_url(self):
    self.adapter.response_url = "https://www.googleapis.com/upload/youtube/v3/videos?changed=1"
    with self.assertRaises(transport.TransportError) as caught:
      transport.create_session(self.session, {"snippet": {}}, 1)
    self.assertEqual(caught.exception.kind, "uncertain")
    self.assertEqual(len(self.adapter.calls), 1)

  def test_unsafe_real_session_configuration_is_rejected_before_send(self):
    cases = (
      lambda session: setattr(session, "_max_refresh_attempts", 1),
      lambda session: setattr(session, "trust_env", True),
      lambda session: session.hooks["response"].append(lambda response, *args, **kwargs: response),
      lambda session: setattr(session.credentials, "_use_non_blocking_refresh", True),
      lambda session: setattr(session.get_adapter(URI).max_retries, "total", 3),
      lambda session: setattr(session._auth_request_session.get_adapter(URI).max_retries, "total", 3),
      lambda session: setattr(session._auth_request, "session", None),
    )
    for mutate in cases:
      with self.subTest(mutate=mutate):
        session = transport.make_session(OfflineCredentials())
        self.addCleanup(session.close)
        adapter = OfflineAdapter()
        session.mount("https://", adapter)
        mutate(session)
        with self.assertRaises(transport.TransportError) as caught:
          transport.query_status(session, URI, 1)
        self.assertEqual(caught.exception.kind, "uncertain")
        self.assertEqual(adapter.calls, [])
    ordinary = AuthorizedSession(OfflineCredentials())
    self.addCleanup(ordinary.close)
    with self.assertRaises(transport.TransportError):
      transport.query_status(ordinary, URI, 1)


if __name__ == "__main__":
  unittest.main()
