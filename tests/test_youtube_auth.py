import contextlib
from concurrent.futures import ThreadPoolExecutor
import ctypes
import datetime
import errno
import fcntl
import importlib.util
import io
import json
import logging
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import threading
import types
import unittest
from unittest import mock
from urllib.parse import parse_qs

try:
  from google_auth_oauthlib.flow import InstalledAppFlow
  from google.oauth2.credentials import Credentials as GoogleCredentials
  from google.oauth2 import reauth as google_reauth
  import requests
  from requests.adapters import HTTPAdapter
except ImportError:
  GoogleCredentials = None
  google_reauth = None
  InstalledAppFlow = None
  requests = None
  HTTPAdapter = object


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "youtube_auth.py"
SPEC = importlib.util.spec_from_file_location("youtube_auth", SCRIPT)
AUTH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUTH)
CLIENT = "synthetic-client.apps.googleusercontent.com"
SECRET = "synthetic-secret"
TOKEN = "synthetic-refresh-token"
CHANNEL = "UC" + "a" * 22


class FakeTokenAdapter(HTTPAdapter):
  def __init__(self, status, body=b"{}"):
    super().__init__(max_retries=0)
    self.status = status
    self.body = body
    self.calls = []

  def send(self, request, **kwargs):
    self.calls.append((request, kwargs))
    logging.getLogger("urllib3.connectionpool").debug(
      "synthetic token %s %s", request.url, request.body)
    logging.getLogger("urllib3.connectionpool").log(
      logging.CRITICAL + 1, "synthetic token %s %s", request.url, request.body)
    response = requests.Response()
    response.status_code = self.status
    response.url = request.url
    response.request = request
    response.headers["Location"] = "https://attacker.invalid/steal"
    response._content = self.body
    return response


class YouTubeAuthTests(unittest.TestCase):
  def setUp(self):
    self.temp = tempfile.TemporaryDirectory()
    self.addCleanup(self.temp.cleanup)
    self.home = Path(self.temp.name).resolve() / "home"
    self.home.mkdir(mode=0o700)
    self.home_patch = mock.patch.object(AUTH.Path, "home", return_value=self.home)
    self.home_patch.start()
    self.addCleanup(self.home_patch.stop)
    self.app = self.home / ".config" / "chipviz"

  def configure(self, values=(CLIENT, SECRET)):
    with mock.patch.object(AUTH.getpass, "getpass", side_effect=values) as prompt:
      AUTH.configure()
    return prompt

  def google_modules(self, refresh_token=TOKEN, response=None):
    credentials = mock.Mock(refresh_token=refresh_token, granted_scopes=AUTH.SCOPES, valid=True)
    flow = mock.Mock()
    flow.run_local_server.return_value = credentials
    flow_class = mock.Mock()
    flow_class.from_client_config.return_value = flow
    service = mock.Mock()
    service.channels.return_value.list.return_value.execute.return_value = response or {
      "items": [{"id": CHANNEL, "snippet": {"title": "Synthetic channel"}}],
      "pageInfo": {"totalResults": 1},
    }
    build = mock.Mock(return_value=service)
    class FakeCredentials:
      constructed = mock.Mock()
      refresh_mock = mock.Mock()

      def __init__(self, **kwargs):
        self.constructed(**kwargs)
        self.refresh_token = kwargs["refresh_token"]
        self.token = None
        self.valid = True
        self.granted_scopes = AUTH.SCOPES

      def refresh(self, request):
        self.refresh_mock(self, request)
        self.token = "synthetic-access-token"

    credentials_class = FakeCredentials
    request = mock.Mock()
    modules = {
      "google_auth_oauthlib": types.ModuleType("google_auth_oauthlib"),
      "google_auth_oauthlib.flow": types.ModuleType("google_auth_oauthlib.flow"),
      "googleapiclient": types.ModuleType("googleapiclient"),
      "googleapiclient.discovery": types.ModuleType("googleapiclient.discovery"),
      "google": types.ModuleType("google"),
      "google.auth": types.ModuleType("google.auth"),
      "google.auth.transport": types.ModuleType("google.auth.transport"),
      "google.auth.transport.requests": types.ModuleType("google.auth.transport.requests"),
      "google.oauth2": types.ModuleType("google.oauth2"),
      "google.oauth2.credentials": types.ModuleType("google.oauth2.credentials"),
      "requests": types.ModuleType("requests"),
      "requests.adapters": types.ModuleType("requests.adapters"),
    }
    modules["google_auth_oauthlib.flow"].InstalledAppFlow = flow_class
    modules["googleapiclient.discovery"].build = build
    modules["google.auth.transport.requests"].Request = request
    modules["google.oauth2.credentials"].Credentials = credentials_class
    modules["requests.adapters"].HTTPAdapter = mock.Mock()
    return modules, credentials, flow_class, flow, service, credentials_class

  def test_configure_uses_getpass_and_private_exact_permissions(self):
    output = io.StringIO()
    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
      prompt = self.configure()
    self.assertEqual(prompt.call_count, 2)
    self.assertNotIn(CLIENT, output.getvalue())
    self.assertNotIn(SECRET, output.getvalue())
    self.assertEqual(stat.S_IMODE(self.app.stat().st_mode), 0o700)
    env = self.app / AUTH.ENV_FILE
    self.assertEqual(stat.S_IMODE(env.stat().st_mode), 0o600)
    self.assertEqual(env.read_text(), f"YOUTUBE_CLIENT_ID={CLIENT}\nYOUTUBE_CLIENT_SECRET={SECRET}\n")

  def test_no_terminal_fallback_does_not_read_or_echo(self):
    def warning(_prompt):
      import warnings
      warnings.warn("Cannot disable echo", AUTH.getpass.GetPassWarning)
      self.fail("fallback read would echo")
    with mock.patch.object(AUTH.getpass, "getpass", side_effect=warning):
      with self.assertRaises(AUTH.AuthError):
        AUTH.configure()
    self.assertFalse((self.app / AUTH.ENV_FILE).exists())

  def test_invalid_cli_arguments_are_never_echoed(self):
    output = io.StringIO()
    with mock.patch.object(sys, "argv", ["youtube_auth.py", "configure", SECRET]), \
        contextlib.redirect_stderr(output):
      with self.assertRaises(SystemExit) as result:
        AUTH.main()
    self.assertEqual(result.exception.code, 2)
    self.assertNotIn(SECRET, output.getvalue())

  def test_unverifiable_acl_fails_closed(self):
    with mock.patch.object(AUTH, "_check_acl", side_effect=AUTH.AuthError("ACL")):
      with self.assertRaises(AUTH.AuthError):
        self.configure()
    self.assertFalse(self.app.exists())

  def test_existing_file_never_overwritten_or_prompted(self):
    self.configure()
    env = self.app / AUTH.ENV_FILE
    original = env.read_bytes()
    with mock.patch.object(AUTH.getpass, "getpass") as prompt:
      with self.assertRaises(AUTH.AuthError):
        AUTH.configure()
    prompt.assert_not_called()
    self.assertEqual(env.read_bytes(), original)

  def test_symlinks_and_insecure_permissions_are_rejected(self):
    self.configure()
    env = self.app / AUTH.ENV_FILE
    env.unlink()
    target = self.home / "other"
    target.write_text("unchanged")
    env.symlink_to(target)
    with self.assertRaises(AUTH.AuthError):
      AUTH.configure()
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()
    self.assertEqual(target.read_text(), "unchanged")
    env.unlink()
    self.app.chmod(0o755)
    with self.assertRaises(AUTH.AuthError):
      self.configure()
    self.assertEqual(stat.S_IMODE(self.app.stat().st_mode), 0o755)
    self.app.chmod(0o700)
    (self.app / AUTH.LOCK_FILE).unlink()
    self.app.rmdir()
    self.app.symlink_to(self.home, target_is_directory=True)
    with self.assertRaises(AUTH.AuthError):
      self.configure()

  def test_in_git_worktree_is_rejected_before_creation(self):
    (self.home / ".git").write_text("gitdir: synthetic\n")
    with self.assertRaises(AUTH.AuthError):
      self.configure()
    self.assertFalse(self.app.exists())

  def test_malformed_values_are_not_stored(self):
    for values in [
      ("not-an-id", SECRET), (CLIENT, "bad\nSECRET=leak"),
      (CLIENT, "x"), ("bad id.apps.googleusercontent.com", SECRET),
    ]:
      with self.subTest(values=values):
        with self.assertRaises(AUTH.AuthError):
          self.configure(values)
        self.assertFalse((self.app / AUTH.ENV_FILE).exists())

  def test_authorize_confirms_exact_channel_and_loads_refreshed_credentials(self):
    self.configure()
    modules, credentials, flow_class, flow, service, credentials_class = self.google_modules()
    output = io.StringIO()
    with mock.patch.dict(sys.modules, modules), mock.patch("builtins.input", return_value=CHANNEL), \
        contextlib.redirect_stdout(output):
      AUTH.authorize()
      loaded, channel = AUTH.load_credentials()
    self.assertIsInstance(loaded, credentials_class)
    self.assertEqual(channel, CHANNEL)
    flow_class.from_client_config.assert_called_once()
    self.assertEqual(flow_class.from_client_config.call_args.kwargs["scopes"], AUTH.SCOPES)
    self.assertTrue(flow_class.from_client_config.call_args.kwargs["autogenerate_code_verifier"])
    self.assertFalse(flow.oauth2session.trust_env)
    self.assertEqual(flow.oauth2session.mount.call_count, 2)
    flow.oauth2session.close.assert_called_once()
    self.assertEqual(flow.run_local_server.call_args.kwargs["host"], "127.0.0.1")
    self.assertTrue(flow.run_local_server.call_args.kwargs["open_browser"])
    service.channels.return_value.list.assert_called_once_with(
      part="snippet", mine=True, maxResults=2)
    credentials_class.constructed.assert_called_once_with(
      token=None, refresh_token=TOKEN, token_uri=AUTH.TOKEN_URI,
      client_id=CLIENT, client_secret=SECRET, scopes=AUTH.SCOPES)
    credentials_class.refresh_mock.assert_called_once()
    state = self.app / AUTH.STATE_FILE
    self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o600)
    self.assertEqual(json.loads(state.read_text()), {"channel_id": CHANNEL, "refresh_token": TOKEN})
    self.assertIn(CHANNEL, output.getvalue())
    for value in (CLIENT, SECRET, TOKEN):
      self.assertNotIn(value, output.getvalue())

  @unittest.skipUnless(InstalledAppFlow is not None, "optional Google dependencies not installed")
  def test_real_code_exchange_blocks_redirects_and_preserves_pkce(self):
    for status in (301, 302, 303, 307, 308):
      with self.subTest(status=status):
        config = {"installed": {"client_id": CLIENT, "client_secret": SECRET,
                                "auth_uri": AUTH.AUTH_URI, "token_uri": AUTH.TOKEN_URI,
                                "redirect_uris": ["http://localhost"]}}
        flow = InstalledAppFlow.from_client_config(
          config, scopes=AUTH.SCOPES, autogenerate_code_verifier=True)
        flow.redirect_uri = "http://127.0.0.1:12345/"
        url, _ = flow.authorization_url()
        self.assertIn("code_challenge=", url)
        AUTH._protect_token_session(flow.oauth2session)
        adapter = FakeTokenAdapter(status)
        flow.oauth2session.mount("https://", adapter)
        output = io.StringIO()
        handler = logging.StreamHandler(output)
        logger = logging.getLogger("urllib3.connectionpool")
        previous = logger.level
        logger.setLevel(logging.DEBUG)
        logger.addHandler(handler)
        try:
          with mock.patch.dict(os.environ, {"HTTPS_PROXY": "http://attacker.invalid:1234"}), \
              contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            with self.assertRaises(AUTH.AuthError) as error, AUTH._quiet_google():
              flow.fetch_token(code="synthetic-code")
        finally:
          logger.removeHandler(handler)
          logger.setLevel(previous)
          flow.oauth2session.close()
        self.assertEqual(error.exception.kind, "uncertain")
        self.assertNotIn("attacker.invalid", str(error.exception))
        self.assertEqual(len(adapter.calls), 1)
        request, kwargs = adapter.calls[0]
        self.assertEqual(request.url, AUTH.TOKEN_URI)
        self.assertEqual(request.method, "POST")
        self.assertEqual(parse_qs(request.body)["code_verifier"], [flow.code_verifier])
        self.assertFalse(kwargs["proxies"])
        self.assertEqual(kwargs["timeout"], AUTH.TOKEN_TIMEOUT)
        for secret in (CLIENT, SECRET, "synthetic-code", flow.code_verifier, "attacker.invalid"):
          self.assertNotIn(secret, output.getvalue())

  @unittest.skipUnless(InstalledAppFlow is not None, "optional Google dependencies not installed")
  def test_real_code_exchange_succeeds_with_pkce(self):
    config = {"installed": {"client_id": CLIENT, "client_secret": SECRET,
                            "auth_uri": AUTH.AUTH_URI, "token_uri": AUTH.TOKEN_URI,
                            "redirect_uris": ["http://localhost"]}}
    flow = InstalledAppFlow.from_client_config(
      config, scopes=AUTH.SCOPES, autogenerate_code_verifier=True)
    flow.redirect_uri = "http://127.0.0.1:12345/"
    flow.authorization_url()
    AUTH._protect_token_session(flow.oauth2session)
    adapter = FakeTokenAdapter(200, json.dumps({
      "access_token": "synthetic-access", "refresh_token": TOKEN, "expires_in": 3600,
      "scope": " ".join(AUTH.SCOPES), "token_type": "Bearer",
    }).encode())
    flow.oauth2session.mount("https://", adapter)
    try:
      with AUTH._quiet_google():
        flow.fetch_token(code="synthetic-code")
      self.assertEqual(flow.credentials.refresh_token, TOKEN)
      self.assertEqual(parse_qs(adapter.calls[0][0].body)["code_verifier"], [flow.code_verifier])
      self.assertEqual(len(adapter.calls), 1)
    finally:
      flow.oauth2session.close()

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_real_initial_refresh_blocks_all_redirects(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    output = io.StringIO()
    handler = logging.StreamHandler(output)
    logger = logging.getLogger("urllib3.connectionpool")
    previous = logger.level
    logger.setLevel(logging.DEBUG)
    logger.addHandler(handler)
    self.addCleanup(logger.removeHandler, handler)
    self.addCleanup(logger.setLevel, previous)
    for status in (301, 302, 303, 307, 308):
      with self.subTest(status=status):
        adapter = FakeTokenAdapter(status)
        def send(_self, request, **kwargs):
          return adapter.send(request, **kwargs)
        with mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send), \
            mock.patch.dict(os.environ, {"HTTPS_PROXY": "http://attacker.invalid:1234"}), \
            contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
          with self.assertRaises(AUTH.AuthError) as error:
            AUTH.load_credentials()
        self.assertEqual(error.exception.kind, "uncertain")
        self.assertNotIn(TOKEN, str(error.exception))
        self.assertEqual(len(adapter.calls), 1)
        request, kwargs = adapter.calls[0]
        self.assertEqual(request.url, AUTH.TOKEN_URI)
        self.assertEqual(request.method, "POST")
        self.assertFalse(kwargs["proxies"])
        self.assertEqual(kwargs["timeout"], AUTH.TOKEN_TIMEOUT)
        self.assertEqual(json.loads(state.read_text())["refresh_token"], TOKEN)
    for secret in (TOKEN, SECRET, AUTH.TOKEN_URI, "attacker.invalid"):
      self.assertNotIn(secret, output.getvalue())

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_token_guard_rejects_wrong_endpoint_or_method_before_send(self):
    for method, url in (
      ("POST", "https://attacker.invalid/token"),
      ("POST", "http://oauth2.googleapis.com/token"),
      ("POST", AUTH.TOKEN_URI + "?other=1"),
      ("GET", AUTH.TOKEN_URI),
    ):
      with self.subTest(method=method, url=url):
        session = requests.Session()
        AUTH._protect_token_session(session)
        adapter = FakeTokenAdapter(200)
        session.mount("https://", adapter)
        try:
          with self.assertRaises(AUTH.AuthError):
            session.request(method, url, data=TOKEN)
          self.assertEqual(adapter.calls, [])
        finally:
          session.close()

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_token_guard_rejects_second_attempt(self):
    session = requests.Session()
    AUTH._protect_token_session(session)
    adapter = FakeTokenAdapter(200)
    session.mount("https://", adapter)
    try:
      with AUTH._quiet_google():
        session.request("POST", AUTH.TOKEN_URI, data=TOKEN)
        with self.assertRaises(AUTH.AuthError):
          session.request("POST", AUTH.TOKEN_URI, data=TOKEN)
      self.assertEqual(len(adapter.calls), 1)
    finally:
      session.close()

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_real_initial_refresh_preserves_definitive_revocation(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    adapter = FakeTokenAdapter(400, b'{"error":"invalid_grant"}')
    def send(_self, request, **kwargs):
      return adapter.send(request, **kwargs)
    with mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send):
      with self.assertRaises(AUTH.AuthError) as error:
        AUTH.load_credentials()
    self.assertEqual(error.exception.kind, "auth_required")
    self.assertEqual(len(adapter.calls), 1)
    self.assertEqual(json.loads(state.read_text())["refresh_token"], TOKEN)

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_channel_confirmation_refresh_also_blocks_redirects(self):
    self.configure()
    credentials = GoogleCredentials(
      token=None, refresh_token=TOKEN, token_uri=AUTH.TOKEN_URI,
      client_id=CLIENT, client_secret=SECRET, scopes=AUTH.SCOPES)
    flow = mock.Mock()
    flow.oauth2session = requests.Session()
    flow.run_local_server.return_value = credentials
    service = mock.Mock()
    def confirm():
      credentials.refresh(object())
      return {"items": [{"id": CHANNEL, "snippet": {"title": "Synthetic"}}]}
    service.channels.return_value.list.return_value.execute.side_effect = confirm
    adapter = FakeTokenAdapter(307)
    def send(_self, request, **kwargs):
      return adapter.send(request, **kwargs)
    with mock.patch.object(InstalledAppFlow, "from_client_config", return_value=flow), \
        mock.patch("googleapiclient.discovery.build", return_value=service), \
        mock.patch.object(HTTPAdapter, "send", autospec=True, side_effect=send), \
        contextlib.redirect_stdout(io.StringIO()):
      with self.assertRaises(AUTH.AuthError) as error:
        AUTH.authorize()
    self.assertEqual(error.exception.kind, "uncertain")
    self.assertEqual(len(adapter.calls), 1)
    self.assertEqual(adapter.calls[0][0].url, AUTH.TOKEN_URI)
    self.assertFalse((self.app / AUTH.STATE_FILE).exists())

  def test_unconfirmed_or_ambiguous_channel_and_no_refresh_token(self):
    self.configure()
    cases = [
      (None, None, CHANNEL),
      (TOKEN, None, "yes"),
      (TOKEN, {"items": []}, CHANNEL),
      (TOKEN, {"items": [{"id": CHANNEL, "snippet": {"title": "X"}}],
               "nextPageToken": "more"}, CHANNEL),
    ]
    for token, response, confirmation in cases:
      with self.subTest(token=token, response=response, confirmation=confirmation):
        modules, *_ = self.google_modules(token, response)
        with mock.patch.dict(sys.modules, modules), \
            mock.patch("builtins.input", return_value=confirmation), \
            contextlib.redirect_stdout(io.StringIO()):
          with self.assertRaises(AUTH.AuthError):
            AUTH.authorize()
        self.assertFalse((self.app / AUTH.STATE_FILE).exists())

  def test_revoked_unsafe_and_missing_state_fail_without_secret_errors(self):
    self.configure()
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()
    state = self.app / AUTH.STATE_FILE
    state.write_text(json.dumps({"channel_id": CHANNEL, "refresh_token": TOKEN}))
    state.chmod(0o600)
    modules, _, _, _, _, credentials_class = self.google_modules()
    credentials_class.refresh_mock.side_effect = ValueError(TOKEN)
    with mock.patch.dict(sys.modules, modules):
      with self.assertRaises(AUTH.AuthError) as error:
        AUTH.load_credentials()
    self.assertNotIn(TOKEN, str(error.exception))
    self.assertEqual(error.exception.kind, "uncertain")
    state.chmod(0o644)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()
    state.unlink()
    state.symlink_to(self.app / AUTH.ENV_FILE)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()

  def test_loading_preserves_safe_refresh_failure_kind(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    for failure, kind in (
      (AUTH.AuthError(TOKEN, kind="auth_required"), "auth_required"),
      (AUTH.AuthError(TOKEN), "uncertain"),
      (OSError(TOKEN), "uncertain"),
    ):
      with self.subTest(kind=kind, failure=type(failure)):
        modules, _, _, _, _, credentials_class = self.google_modules()
        credentials_class.refresh_mock.side_effect = failure
        with mock.patch.dict(sys.modules, modules):
          with self.assertRaises(AUTH.AuthError) as error:
            AUTH.load_credentials()
        self.assertEqual(error.exception.kind, kind)
        self.assertNotIn(TOKEN, str(error.exception))

  def test_oauth_errors_and_logs_do_not_expose_secrets(self):
    self.configure()
    modules, _, _, flow, _, _ = self.google_modules()
    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    logger = logging.getLogger("synthetic-oauth")
    logger.addHandler(handler)
    self.addCleanup(logger.removeHandler, handler)
    def failing_flow(**_kwargs):
      logging.getLogger("synthetic-oauth").warning(SECRET)
      print(TOKEN)
      raise ValueError(CLIENT)
    flow.run_local_server.side_effect = failing_flow
    with mock.patch.dict(sys.modules, modules), contextlib.redirect_stdout(stream), \
        contextlib.redirect_stderr(stream):
      with self.assertRaises(AUTH.AuthError) as error:
        AUTH.authorize()
    for value in (CLIENT, SECRET, TOKEN):
      self.assertNotIn(value, stream.getvalue())
      self.assertNotIn(value, str(error.exception))

  def test_missing_scope_or_existing_authorization_rejected(self):
    self.configure()
    modules, credentials, _, _, _, _ = self.google_modules()
    credentials.granted_scopes = [AUTH.SCOPES[0]]
    with mock.patch.dict(sys.modules, modules), contextlib.redirect_stdout(io.StringIO()):
      with self.assertRaises(AUTH.AuthError):
        AUTH.authorize()
    self.assertFalse((self.app / AUTH.STATE_FILE).exists())
    state = self.app / AUTH.STATE_FILE
    state.write_text("unchanged")
    state.chmod(0o600)
    with mock.patch.dict(sys.modules, modules):
      with self.assertRaises(AUTH.AuthError):
        AUTH.authorize()
    self.assertEqual(state.read_text(), "unchanged")

  def test_atomic_no_replace_survives_interruption(self):
    self.home.joinpath(".config").mkdir(mode=0o700)
    self.app.mkdir(mode=0o700)
    fd = os.open(self.app, os.O_RDONLY | os.O_DIRECTORY)
    self.addCleanup(os.close, fd)
    for after_rename in (False, True):
      target = "after" if after_rename else "before"
      pid = os.fork()
      if pid == 0:
        original = AUTH._rename_noreplace
        def interrupted(*args):
          if after_rename:
            original(*args)
          os._exit(47)
        AUTH._rename_noreplace = interrupted
        AUTH._write_once(fd, target, "synthetic-secret")
        os._exit(48)
      _, status = os.waitpid(pid, 0)
      self.assertEqual(os.waitstatus_to_exitcode(status), 47)
      path = self.app / target
      self.assertEqual(path.exists(), after_rename)
      if after_rename:
        self.assertEqual(path.stat().st_nlink, 1)
        with self.assertRaises(FileExistsError):
          AUTH._write_once(fd, target, "replacement")
        self.assertEqual(path.read_text(), "synthetic-secret")
      else:
        orphan = list(self.app.glob(".youtube-*"))
        self.assertEqual(len(orphan), 1)
        self.assertEqual(stat.S_IMODE(orphan[0].stat().st_mode), 0o600)
        self.assertEqual(orphan[0].stat().st_nlink, 1)
        AUTH._write_once(fd, target, "retry")
        self.assertEqual(path.read_text(), "retry")

  def test_rotation_persists_on_initial_and_later_refresh(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    modules, _, _, _, _, credentials_class = self.google_modules()
    rotations = iter(["synthetic-rotation-one", "synthetic-rotation-two"])
    def rotate(credentials, _request):
      credentials.refresh_token = next(rotations)
    credentials_class.refresh_mock.side_effect = rotate
    with mock.patch.dict(sys.modules, modules):
      credentials, channel = AUTH.load_credentials()
      self.assertEqual(json.loads(state.read_text())["refresh_token"], "synthetic-rotation-one")
      credentials.refresh(object())
    self.assertEqual(channel, CHANNEL)
    self.assertEqual(json.loads(state.read_text())["refresh_token"], "synthetic-rotation-two")
    self.assertEqual(state.stat().st_nlink, 1)
    self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o600)

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_real_google_credentials_automatic_refresh_persists_rotation(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None) + datetime.timedelta(hours=1)
    responses = [
      ("synthetic-access-one", "synthetic-rotation-one", expiry, {}, None),
      ("synthetic-access-two", "synthetic-rotation-two", expiry, {}, None),
    ]
    with mock.patch.object(google_reauth, "refresh_grant", side_effect=responses) as grant:
      credentials, channel = AUTH.load_credentials()
      self.assertIsInstance(credentials, GoogleCredentials)
      credentials.expiry = expiry - datetime.timedelta(hours=2)
      headers = {}
      credentials.before_request(mock.Mock(), "GET", "https://www.googleapis.com/", headers)
    self.assertEqual(grant.call_count, 2)
    self.assertEqual(channel, CHANNEL)
    self.assertEqual(headers["authorization"], "Bearer synthetic-access-two")
    self.assertEqual(json.loads(state.read_text())["refresh_token"], "synthetic-rotation-two")

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_real_google_refresh_does_not_retry_token_endpoint_5xx(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None) + datetime.timedelta(hours=1)
    initial = ("synthetic-access", TOKEN, expiry, {}, None)
    with mock.patch.object(google_reauth, "refresh_grant", return_value=initial):
      credentials, _ = AUTH.load_credentials()
    response = types.SimpleNamespace(status=503, data=b'{"error":"server_error"}')
    request = mock.Mock(return_value=response)
    with mock.patch.object(AUTH, "_token_request", return_value=contextlib.nullcontext(request)):
      with self.assertRaises(AUTH.AuthError) as error:
        credentials.refresh(request)
    self.assertEqual(request.call_count, 1)
    self.assertEqual(error.exception.kind, "uncertain")
    self.assertIsNone(credentials.token)
    self.assertEqual(json.loads(state.read_text())["refresh_token"], TOKEN)

  @unittest.skipUnless(GoogleCredentials is not None, "optional Google dependencies not installed")
  def test_real_google_refresh_classifies_only_definitive_invalid_grant(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    expiry = datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None) + datetime.timedelta(hours=1)
    initial = ("synthetic-access", TOKEN, expiry, {}, None)
    cases = (
      (400, b'{"error":"invalid_grant","error_description":"synthetic-refresh-token"}',
       "auth_required"),
      (503, b'{"error":"invalid_grant"}', "uncertain"),
      (400, b'{"error":"temporarily_unavailable"}', "uncertain"),
      (400, b"synthetic-refresh-token", "uncertain"),
    )
    for status, body, kind in cases:
      with self.subTest(status=status, kind=kind):
        with mock.patch.object(google_reauth, "refresh_grant", return_value=initial):
          credentials, _ = AUTH.load_credentials()
        request = mock.Mock(return_value=types.SimpleNamespace(status=status, data=body))
        with mock.patch.object(AUTH, "_token_request", return_value=contextlib.nullcontext(request)):
          with self.assertRaises(AUTH.AuthError) as error:
            credentials.refresh(request)
        self.assertEqual(error.exception.kind, kind)
        self.assertNotIn(TOKEN, str(error.exception))
        self.assertEqual(request.call_count, 1)
        self.assertIsNone(credentials.token)

  def test_rotation_serializes_and_stale_credentials_fail_closed(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    modules, _, _, _, _, credentials_class = self.google_modules()
    with mock.patch.dict(sys.modules, modules):
      first, _ = AUTH.load_credentials()
      second, _ = AUTH.load_credentials()
      started = threading.Event()
      release = threading.Event()
      def rotate(credentials, _request):
        started.set()
        if not release.wait(10):
          raise RuntimeError("test timed out")
        credentials.refresh_token = "synthetic-rotated-token"
      credentials_class.refresh_mock.side_effect = rotate
      with ThreadPoolExecutor(max_workers=2) as pool:
        future = pool.submit(first.refresh, object())
        self.assertTrue(started.wait(10))
        lock_fd = os.open(self.app / AUTH.LOCK_FILE, os.O_RDWR | os.O_NOFOLLOW)
        try:
          with self.assertRaises(BlockingIOError):
            fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        finally:
          os.close(lock_fd)
        stale = pool.submit(second.refresh, object())
        release.set()
        future.result(timeout=10)
        with self.assertRaises(AUTH.AuthError):
          stale.result(timeout=10)
    self.assertIsNone(second.token)
    self.assertEqual(json.loads(state.read_text())["refresh_token"], "synthetic-rotated-token")

  def test_failed_rotation_write_invalidates_credentials(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    modules, _, _, _, _, credentials_class = self.google_modules()
    with mock.patch.dict(sys.modules, modules):
      credentials, _ = AUTH.load_credentials()
      def rotate(cred, _request):
        cred.refresh_token = "synthetic-rotated-token"
      credentials_class.refresh_mock.side_effect = rotate
      with mock.patch.object(AUTH, "_atomic_write", side_effect=OSError(TOKEN)):
        with self.assertRaises(AUTH.AuthError) as error:
          credentials.refresh(object())
      self.assertNotIn(TOKEN, str(error.exception))
      self.assertEqual(error.exception.kind, "uncertain")
      self.assertIsNone(credentials.token)
      self.assertEqual(json.loads(state.read_text())["refresh_token"], TOKEN)
      with self.assertRaises(AUTH.AuthError) as stale:
        credentials.refresh(object())
      self.assertEqual(stale.exception.kind, "uncertain")

  def test_ambiguous_published_rotation_cannot_refresh_again(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    modules, _, _, _, _, credentials_class = self.google_modules()
    original_write = AUTH._atomic_write
    with mock.patch.dict(sys.modules, modules):
      credentials, _ = AUTH.load_credentials()
      def rotate(cred, _request):
        cred.refresh_token = "synthetic-rotated-token"
      credentials_class.refresh_mock.side_effect = rotate
      def publish_then_fail(*args, **kwargs):
        original_write(*args, **kwargs)
        raise OSError("synthetic-refresh-token")
      with mock.patch.object(AUTH, "_atomic_write", side_effect=publish_then_fail):
        with self.assertRaises(AUTH.AuthError) as error:
          credentials.refresh(object())
      self.assertEqual(error.exception.kind, "uncertain")
      self.assertEqual(json.loads(state.read_text())["refresh_token"], "synthetic-rotated-token")
      calls = credentials_class.refresh_mock.call_count
      with self.assertRaises(AUTH.AuthError) as error:
        credentials.refresh(object())
      self.assertEqual(error.exception.kind, "uncertain")
      self.assertEqual(credentials_class.refresh_mock.call_count, calls)
      self.assertIsNone(credentials.token)

  def test_unsafe_lock_is_rejected_without_touching_target(self):
    self.configure()
    state = self.app / AUTH.STATE_FILE
    state.write_text(AUTH._state_text(CHANNEL, TOKEN))
    state.chmod(0o600)
    target = self.home / "unrelated"
    target.write_text("unchanged")
    lock = self.app / AUTH.LOCK_FILE
    lock.symlink_to(target)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()
    self.assertEqual(target.read_text(), "unchanged")
    lock.unlink()
    lock.write_text("")
    lock.chmod(0o644)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()

  @unittest.skipUnless(sys.platform == "darwin", "requires Darwin ACL support")
  def test_real_darwin_deny_ancestor_and_private_acls(self):
    def chmod_acl(path, *args):
      subprocess.run(["/bin/chmod", *args, str(path)], check=True,
                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
      chmod_acl(self.home, "+a", "group:everyone deny delete")
      self.configure()
      chmod_acl(self.app, "+a", "group:everyone deny delete")
      with self.assertRaises(AUTH.AuthError):
        AUTH.load_credentials()
      chmod_acl(self.app, "-N")
      env = self.app / AUTH.ENV_FILE
      chmod_acl(env, "+a", "group:everyone deny delete")
      with self.assertRaises(AUTH.AuthError):
        AUTH.load_credentials()
      chmod_acl(env, "-N")
      chmod_acl(self.home, "+a", "group:everyone allow read")
      with self.assertRaises(AUTH.AuthError):
        AUTH.load_credentials()
    finally:
      for path in (self.home, self.app, self.app / AUTH.ENV_FILE):
        if path.exists():
          subprocess.run(["/bin/chmod", "-N", str(path)], check=True,
                         stdout=subprocess.PIPE, stderr=subprocess.PIPE)

  def test_darwin_acl_fake_mixed_and_errors_fail_closed(self):
    def fake_lib(tags, fail_at=None):
      lib = mock.Mock()
      lib.acl_get_fd_np.return_value = 1234
      lib.acl_valid.return_value = 0
      index = [-1]
      def get_entry(_acl, which, pointer):
        if fail_at == which:
          return -1
        if which == -2:
          value = len(tags)
        elif which == 0:
          index[0] = 0
          value = 1
        else:
          index[0] += 1
          if index[0] >= len(tags):
            ctypes.set_errno(errno.EINVAL)
            return -1
          value = index[0] + 1
        ctypes.cast(pointer, ctypes.POINTER(ctypes.c_void_p))[0] = value
        return 0
      def get_tag(entry, pointer):
        ctypes.cast(pointer, ctypes.POINTER(ctypes.c_int))[0] = tags[entry.value - 1]
        return 0
      lib.acl_get_entry.side_effect = get_entry
      lib.acl_get_tag_type.side_effect = get_tag
      return lib

    with mock.patch.object(AUTH.sys, "platform", "darwin"):
      for tags, error in [([2], False), ([2, 2], False), ([2, 1], True),
                          ([1, 2], True), ([2, 99], True)]:
        with self.subTest(tags=tags):
          lib = fake_lib(tags)
          with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
            if error:
              with self.assertRaises(AUTH.AuthError):
                AUTH._check_acl(1, allow_denies=True)
            else:
              AUTH._check_acl(1, allow_denies=True)
            lib.acl_free.assert_called_once_with(1234)
      for failure in (-2, 0, -1):
        lib = fake_lib([2, 2], fail_at=failure)
        with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
          with self.assertRaises(AUTH.AuthError):
            AUTH._check_acl(1, allow_denies=True)
          lib.acl_free.assert_called_once_with(1234)
      lib = fake_lib([2])
      lib.acl_valid.return_value = -1
      with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
        with self.assertRaises(AUTH.AuthError):
          AUTH._check_acl(1, allow_denies=True)
      lib = fake_lib([2])
      with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
        with self.assertRaises(AUTH.AuthError):
          AUTH._check_acl(1)
      lib = fake_lib([2, 2])
      lib.acl_get_tag_type.return_value = -1
      lib.acl_get_tag_type.side_effect = None
      with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
        with self.assertRaises(AUTH.AuthError):
          AUTH._check_acl(1, allow_denies=True)
      lib = fake_lib([2])
      def fail_read(*_args):
        ctypes.set_errno(errno.EACCES)
        return None
      lib.acl_get_fd_np.side_effect = fail_read
      with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
        with self.assertRaises(AUTH.AuthError):
          AUTH._check_acl(1, allow_denies=True)
      lib = fake_lib([2, 1])
      original_get_entry = lib.acl_get_entry.side_effect
      def aliased_last(acl, which, pointer):
        result = original_get_entry(acl, which, pointer)
        if which == -2:
          ctypes.cast(pointer, ctypes.POINTER(ctypes.c_void_p))[0] = 1
        return result
      lib.acl_get_entry.side_effect = aliased_last
      with mock.patch.object(AUTH.ctypes, "CDLL", return_value=lib):
        with self.assertRaises(AUTH.AuthError):
          AUTH._check_acl(1, allow_denies=True)

  def test_linux_acl_and_inspection_errors_fail_closed(self):
    with mock.patch.object(AUTH.sys, "platform", "linux"), \
        mock.patch.object(AUTH.os, "getxattr", return_value=b"acl", create=True):
      with self.assertRaises(AUTH.AuthError):
        AUTH._check_acl(1, allow_denies=True)
    with mock.patch.object(AUTH.sys, "platform", "linux"), \
        mock.patch.object(AUTH.os, "getxattr", side_effect=OSError(errno.EACCES, "denied"),
                          create=True):
      with self.assertRaises(AUTH.AuthError):
        AUTH._check_acl(1, allow_denies=True)
    with mock.patch.object(AUTH.sys, "platform", "linux"), \
        mock.patch.object(AUTH.os, "getxattr", side_effect=OSError(errno.ENOTSUP, "unsupported"),
                          create=True):
      with self.assertRaises(AUTH.AuthError):
        AUTH._check_acl(1, allow_denies=True)
    with mock.patch.object(AUTH.sys, "platform", "linux"), \
        mock.patch.object(AUTH.os, "getxattr", side_effect=OSError(errno.ENODATA, "no ACL"),
                          create=True):
      AUTH._check_acl(1, allow_denies=True)


if __name__ == "__main__":
  unittest.main()
