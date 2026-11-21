import contextlib
import importlib.util
import io
import json
import pathlib
import socket
import stat
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
  "genesis_host_preflight", ROOT / "scripts/genesis_host_preflight.py"
)
preflight = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(preflight)
ENV = {
  "GITHUB_ACTIONS": "true",
  "RUNNER_ENVIRONMENT": "github-hosted",
  "RUNNER_ARCH": "ARM64",
  "GITHUB_REPOSITORY": "djensenius/chipviz",
  "GITHUB_EVENT_NAME": "pull_request",
}
INFO = {
  "OSType": "linux",
  "Architecture": "arm64",
  "ServerVersion": "28.0.4",
  "DockerRootDir": "/var/lib/docker",
  "HTTPProxy": "secret-password",
  "Name": "private-host-name",
}
SNAPSHOT = (123, 10, 20, 0o140660, 0, 999, "42")


def frame(payload):
  return (
    b"HTTP/1.1 200 OK\r\nContent-Length: " + str(len(payload)).encode()
    + b"\r\n\r\n" + payload
  )


class FakeHost:
  def __init__(self):
    self.env = dict(ENV)
    self.arch = "aarch64"
    self.disk = preflight.MIN_FREE
    self.snapshots = [SNAPSHOT, SNAPSHOT, SNAPSHOT]
    self.peer_info = (123, 0, 999)
    self.data = frame(json.dumps(INFO).encode())
    self.calls = []

  def environment(self):
    return self.env

  def machine(self):
    return self.arch

  def free(self, path):
    self.calls.append(("free", path))
    return self.disk

  def snapshot(self):
    self.calls.append(("snapshot",))
    value = self.snapshots.pop(0)
    if isinstance(value, Exception):
      raise value
    return value

  def connect(self):
    self.calls.append(("connect",))
    return contextlib.nullcontext(object())

  def peer(self, client):
    self.calls.append(("peer",))
    return self.peer_info

  def response(self, client):
    self.calls.append(("response",))
    if isinstance(self.data, Exception):
      raise self.data
    return self.data


class PreflightTests(unittest.TestCase):
  def test_good_collection_and_allowlisted_output(self):
    fake = FakeHost()
    with mock.patch.object(preflight, "Host", return_value=fake):
      with contextlib.redirect_stdout(io.StringIO()) as output:
        self.assertEqual(preflight.main(), 0)
    self.assertEqual(output.getvalue(), (
      "host metadata: COLLECTED\n"
      "host architecture: arm64\n"
      "daemon socket/service/peer: MATCHED\n"
      "daemon architecture: arm64\n"
      "daemon version: 28.0.4\n"
      "host and daemon-root free space: at least 5 GiB each\n"
      "builder isolation: UNVERIFIED\n"
      "mount isolation: UNVERIFIED\n"
      "toolchain builds authorized: NO\n"
    ))
    self.assertNotIn("secret-password", output.getvalue())
    self.assertNotIn("private-host-name", output.getvalue())
    self.assertEqual(sum(call == ("response",) for call in fake.calls), 1)

  def test_remote_routing_and_unhosted_are_rejected_before_connection(self):
    for key, value in (("DOCKER_HOST", "tcp://secret"), ("DOCKER_CONTEXT", "remote"),
                       ("DOCKER_CONFIG", "/private"), ("RUNNER_ENVIRONMENT", "self-hosted")):
      fake = FakeHost()
      fake.env[key] = value
      with self.subTest(key=key), self.assertRaises(preflight.Rejected):
        preflight.collect(fake)
      self.assertNotIn(("connect",), fake.calls)

  def test_insufficient_disk_is_rejected_before_connection(self):
    fake = FakeHost()
    fake.disk = preflight.MIN_FREE - 1
    with self.assertRaises(preflight.Rejected):
      preflight.collect(fake)
    self.assertNotIn(("connect",), fake.calls)

  def test_missing_ambiguous_and_swapped_socket_are_rejected(self):
    for snapshots in ([FileNotFoundError()], [preflight.Rejected()],
                      [SNAPSHOT, (123, 10, 21, 0o140660, 0, 999, "42")]):
      fake = FakeHost()
      fake.snapshots = snapshots
      with self.subTest(snapshots=snapshots), self.assertRaises((preflight.Rejected, FileNotFoundError)):
        preflight.collect(fake)
      self.assertNotIn(("response",), fake.calls)

  def test_peer_mismatch_activator_or_non_root_rejected(self):
    for peer in ((1, 0, 0), (124, 0, 0), (123, 1000, 0)):
      fake = FakeHost()
      fake.peer_info = peer
      with self.subTest(peer=peer), self.assertRaises(preflight.Rejected):
        preflight.collect(fake)
      self.assertNotIn(("response",), fake.calls)

  def test_service_or_socket_changes_after_response_fail_closed(self):
    fake = FakeHost()
    fake.snapshots[-1] = (124, *SNAPSHOT[1:])
    with self.assertRaises(preflight.Rejected):
      preflight.collect(fake)

  def test_production_snapshot_uses_only_fake_os_data(self):
    directory = types.SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_uid=0)
    endpoint = types.SimpleNamespace(
      st_mode=stat.S_IFSOCK | 0o660, st_uid=0, st_gid=999, st_dev=10, st_ino=20)
    service = types.SimpleNamespace(
      stdout=b"MainPID=123\nActiveState=active\nSubState=running\n")
    net = (
      b"Num RefCount Protocol Flags Type St Inode Path\n"
      b"00: 00000002 00000000 00010000 0001 01 42 /run/docker.sock\n"
    )
    contents = {
      "/proc/123/cgroup": b"0::/system.slice/docker.service\n",
      "/proc/net/unix": net,
    }

    def fake_open(path, mode):
      self.assertEqual(mode, "rb")
      return io.BytesIO(contents[path])

    def fake_link(path):
      return {
        "/proc/123/exe": "/usr/bin/dockerd",
        "/proc/123/fd/7": "socket:[42]",
      }[path]

    with mock.patch.object(preflight.os, "lstat", side_effect=[directory, directory, endpoint]), \
         mock.patch.object(preflight.subprocess, "run", return_value=service) as run, \
         mock.patch.object(preflight.os, "readlink", side_effect=fake_link), \
         mock.patch.object(preflight.os, "listdir", return_value=["7"]), \
         mock.patch("builtins.open", side_effect=fake_open):
      self.assertEqual(preflight.Host().snapshot(), SNAPSHOT)
    self.assertEqual(run.call_args.args[0], [
      "/usr/bin/systemctl", "show", "docker.service",
      "--property=MainPID,ActiveState,SubState"])

  def test_production_connection_uses_fixed_unix_path(self):
    client = mock.MagicMock()
    with mock.patch.object(preflight.socket, "socket", return_value=client) as factory:
      self.assertIs(preflight.Host().connect(), client)
    factory.assert_called_once_with(socket.AF_UNIX, socket.SOCK_STREAM)
    client.connect.assert_called_once_with("/run/docker.sock")

  def test_production_snapshot_rejects_unsafe_socket_before_service_query(self):
    directory = types.SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_uid=0)
    for mode, uid in ((stat.S_IFLNK | 0o777, 0), (stat.S_IFSOCK | 0o666, 0),
                      (stat.S_IFSOCK | 0o660, 1000)):
      endpoint = types.SimpleNamespace(st_mode=mode, st_uid=uid)
      with self.subTest(mode=mode, uid=uid), \
           mock.patch.object(preflight.os, "lstat", side_effect=[directory, directory, endpoint]), \
           mock.patch.object(preflight.subprocess, "run") as run:
        with self.assertRaises(preflight.Rejected):
          preflight.Host().snapshot()
        run.assert_not_called()

  def test_production_snapshot_rejects_bad_executable_cgroup_or_listener(self):
    directory = types.SimpleNamespace(st_mode=stat.S_IFDIR | 0o755, st_uid=0)
    endpoint = types.SimpleNamespace(
      st_mode=stat.S_IFSOCK | 0o660, st_uid=0, st_gid=999, st_dev=10, st_ino=20)
    service = types.SimpleNamespace(
      stdout=b"MainPID=123\nActiveState=active\nSubState=running\n")
    net = (
      b"Num RefCount Protocol Flags Type St Inode Path\n"
      b"00: 00000002 00000000 00010000 0001 01 42 /run/docker.sock\n"
    )
    for executable, cgroup, fd_link in (
      ("/usr/bin/other", b"0::/system.slice/docker.service\n", "socket:[42]"),
      ("/usr/bin/dockerd", b"0::/other.service\n", "socket:[42]"),
      ("/usr/bin/dockerd", b"0::/system.slice/docker.service\n", "socket:[99]"),
    ):
      contents = {"/proc/123/cgroup": cgroup, "/proc/net/unix": net}
      links = {"/proc/123/exe": executable, "/proc/123/fd/7": fd_link}
      with self.subTest(executable=executable, cgroup=cgroup, fd=fd_link), \
           mock.patch.object(preflight.os, "lstat", side_effect=[directory, directory, endpoint]), \
           mock.patch.object(preflight.subprocess, "run", return_value=service), \
           mock.patch.object(preflight.os, "readlink", side_effect=lambda path: links[path]), \
           mock.patch.object(preflight.os, "listdir", return_value=["7"]), \
           mock.patch("builtins.open", side_effect=lambda path, mode: io.BytesIO(contents[path])):
        with self.assertRaises(preflight.Rejected):
          preflight.Host().snapshot()

  def test_service_identity_and_listener_parse(self):
    self.assertEqual(preflight.parse_service(
      b"MainPID=123\nActiveState=active\nSubState=running\n"), 123)
    for service in (b"", b"MainPID=1\nActiveState=active\nSubState=dead\n",
                    b"MainPID=123\nMainPID=456\nActiveState=active\nSubState=running\n",
                    b"MainPID=0\nActiveState=active\nSubState=running\n"):
      with self.subTest(service=service), self.assertRaises(preflight.Rejected):
        preflight.parse_service(service)
    listener = b"Num RefCount Protocol Flags Type St Inode Path\n"
    listener += b"00: 00000002 00000000 00010000 0001 01 42 /run/docker.sock\n"
    self.assertEqual(preflight.listener_inode(listener), "42")
    for invalid in (b"", listener + listener.splitlines()[1] + b"\n",
                    listener.replace(b"00010000", b"00000000"),
                    listener.replace(b"/run/docker.sock", b"/tmp/remote")):
      with self.subTest(invalid=invalid), self.assertRaises(preflight.Rejected):
        preflight.listener_inode(invalid)

  def test_framing_and_sensitive_fields_fail_closed(self):
    payload = json.dumps(INFO).encode()
    chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
    chunked += f"{len(payload):x}\r\n".encode() + payload + b"\r\n0\r\n\r\n"
    self.assertEqual(preflight.parse_info(chunked), "28.0.4")
    bad = (
      b"", frame(payload)[:-1], frame(payload) + b"x",
      b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx",
      b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\nx",
      b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n",
      frame(b'{"OSType":"linux","OSType":"linux"}'),
      frame(b'{"value":NaN}'),
      frame(json.dumps({**INFO, "ServerVersion": "28.0.4 secret-password"}).encode()),
      frame(json.dumps({**INFO, "DockerRootDir": "/private"}).encode()),
      b"x" * (preflight.MAX_RESPONSE + 1),
    )
    for data in bad:
      with self.subTest(data=data[:80]), self.assertRaises((preflight.Rejected, ValueError)):
        preflight.parse_info(data)

  def test_timeout_and_exception_output_are_sanitized(self):
    fake = FakeHost()
    fake.data = socket.timeout("secret-password /private/path")
    with mock.patch.object(preflight, "Host", return_value=fake):
      with contextlib.redirect_stdout(io.StringIO()) as output:
        self.assertEqual(preflight.main(), 1)
    self.assertEqual(output.getvalue(), (
      "host metadata: UNAVAILABLE\n"
      "builder isolation: UNVERIFIED\n"
      "mount isolation: UNVERIFIED\n"
      "toolchain builds authorized: NO\n"
    ))

  def test_response_sends_only_one_fixed_request(self):
    class FakeSocket:
      def __init__(self):
        self.request = None
        self.chunks = [frame(json.dumps(INFO).encode()), b""]

      def settimeout(self, value):
        self.assert_positive = value > 0

      def sendall(self, value):
        self.request = value

      def recv(self, size):
        return self.chunks.pop(0)

    client = FakeSocket()
    self.assertEqual(preflight.parse_info(preflight.Host().response(client)), "28.0.4")
    self.assertEqual(client.request, preflight.REQUEST)
    self.assertTrue(client.assert_positive)

  def test_response_deadline_and_size_are_bounded(self):
    client = mock.MagicMock()
    client.recv.side_effect = socket.timeout("sensitive")
    with self.assertRaises(socket.timeout):
      preflight.Host().response(client)
    client = mock.MagicMock()
    client.recv.side_effect = [b"x" * preflight.MAX_RESPONSE, b"x"]
    with self.assertRaises(preflight.Rejected):
      preflight.Host().response(client)


if __name__ == "__main__":
  unittest.main()
