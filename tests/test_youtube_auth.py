import contextlib
import importlib.util
import io
import json
import logging
import os
from pathlib import Path
import stat
import sys
import tempfile
import types
import unittest
from unittest import mock


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "youtube_auth.py"
SPEC = importlib.util.spec_from_file_location("youtube_auth", SCRIPT)
AUTH = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUTH)
CLIENT = "synthetic-client.apps.googleusercontent.com"
SECRET = "synthetic-secret"
TOKEN = "synthetic-refresh-token"
CHANNEL = "UC" + "a" * 22


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
    credentials_class = mock.Mock(return_value=credentials)
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
    }
    modules["google_auth_oauthlib.flow"].InstalledAppFlow = flow_class
    modules["googleapiclient.discovery"].build = build
    modules["google.auth.transport.requests"].Request = request
    modules["google.oauth2.credentials"].Credentials = credentials_class
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
    self.assertIs(loaded, credentials)
    self.assertEqual(channel, CHANNEL)
    flow_class.from_client_config.assert_called_once()
    self.assertEqual(flow_class.from_client_config.call_args.kwargs["scopes"], AUTH.SCOPES)
    self.assertTrue(flow_class.from_client_config.call_args.kwargs["autogenerate_code_verifier"])
    self.assertEqual(flow.run_local_server.call_args.kwargs["host"], "127.0.0.1")
    self.assertTrue(flow.run_local_server.call_args.kwargs["open_browser"])
    service.channels.return_value.list.assert_called_once_with(
      part="snippet", mine=True, maxResults=2)
    credentials_class.assert_called_once_with(
      token=None, refresh_token=TOKEN, token_uri=AUTH.TOKEN_URI,
      client_id=CLIENT, client_secret=SECRET, scopes=AUTH.SCOPES)
    credentials.refresh.assert_called_once()
    state = self.app / AUTH.STATE_FILE
    self.assertEqual(stat.S_IMODE(state.stat().st_mode), 0o600)
    self.assertEqual(json.loads(state.read_text()), {"channel_id": CHANNEL, "refresh_token": TOKEN})
    self.assertIn(CHANNEL, output.getvalue())
    for value in (CLIENT, SECRET, TOKEN):
      self.assertNotIn(value, output.getvalue())

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
    modules, credentials, *_ = self.google_modules()
    credentials.refresh.side_effect = ValueError(TOKEN)
    with mock.patch.dict(sys.modules, modules):
      with self.assertRaises(AUTH.AuthError) as error:
        AUTH.load_credentials()
    self.assertNotIn(TOKEN, str(error.exception))
    state.chmod(0o644)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()
    state.unlink()
    state.symlink_to(self.app / AUTH.ENV_FILE)
    with self.assertRaises(AUTH.AuthError):
      AUTH.load_credentials()

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


if __name__ == "__main__":
  unittest.main()
