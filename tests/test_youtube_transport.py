import contextlib
import io
import unittest
from unittest import mock

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
                 {"id": 123}, ValueError(URI)):
      with self.subTest(body=body):
        self.assert_kind("uncertain", transport.query_status,
                         Session(Response(200, body=body)), URI, 10)
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


if __name__ == "__main__":
  unittest.main()
