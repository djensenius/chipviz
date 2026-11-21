"""Hosted metadata collection only; never authorizes a toolchain build."""

import contextlib
import json
import os
import platform
import re
import socket
import stat
import struct
import subprocess
import time


SOCKET_PATH = "/run/docker.sock"
MAX_RESPONSE = 1024 * 1024
MAX_HEADER = 16 * 1024
MIN_FREE = 5 * 1024 ** 3
TIMEOUT = 8.0
REQUEST = b"GET /v1.48/info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"


class Rejected(Exception):
  """Untrusted or incomplete evidence; never print its contents."""


STAGES = frozenset((
  "hosted environment", "host resources", "daemon resources",
  "socket filesystem", "daemon service", "daemon process", "daemon cgroup",
  "socket listener", "daemon listener fd", "socket snapshot", "socket connect",
  "socket peer", "socket recheck", "daemon response", "daemon metadata",
  "resource recheck", "internal",
))


class StageError(Rejected):
  def __init__(self, stage):
    self.stage = stage if isinstance(stage, str) and stage in STAGES else "internal"
    super().__init__()


@contextlib.contextmanager
def check_stage(stage):
  try:
    yield
  except StageError:
    raise
  except Exception:
    raise StageError(stage) from None


def require(condition):
  if not condition:
    raise Rejected()


def parse_service(data):
  require(len(data) <= 4096)
  fields = {}
  for line in data.decode("ascii").splitlines():
    key, sep, value = line.partition("=")
    require(sep and key in ("MainPID", "ActiveState", "SubState") and key not in fields)
    fields[key] = value
  require(fields.keys() == {"MainPID", "ActiveState", "SubState"})
  require(fields["ActiveState"] == "active" and fields["SubState"] == "running")
  require(re.fullmatch(r"[1-9][0-9]{0,9}", fields["MainPID"]) is not None)
  return int(fields["MainPID"])


def listener_inode(data):
  require(len(data) <= MAX_RESPONSE)
  matches = []
  for line in data.decode("ascii").splitlines()[1:]:
    fields = line.split()
    if len(fields) == 8 and fields[7] == SOCKET_PATH:
      require(fields[3] == "00010000" and fields[4] == "0001" and fields[5] == "01")
      require(re.fullmatch(r"[0-9]+", fields[6]) is not None)
      matches.append(fields[6])
  require(len(matches) == 1)
  return matches[0]


def decode_frame(data):
  require(len(data) <= MAX_RESPONSE)
  header, sep, body = data.partition(b"\r\n\r\n")
  require(sep and len(header) <= MAX_HEADER)
  lines = header.split(b"\r\n")
  require(lines[0] == b"HTTP/1.1 200 OK")
  headers = {}
  for line in lines[1:]:
    name, colon, value = line.partition(b":")
    require(colon and re.fullmatch(rb"[A-Za-z0-9-]+", name) is not None)
    name = name.lower()
    require(name not in headers and re.fullmatch(rb"[\t\x20-\x7e]*", value) is not None)
    headers[name] = value.strip().lower()
  require(not (b"content-length" in headers and b"transfer-encoding" in headers))
  if b"content-length" in headers:
    length = headers[b"content-length"]
    require(re.fullmatch(rb"[0-9]{1,7}", length) is not None)
    require(int(length) == len(body))
    return body
  require(headers.get(b"transfer-encoding") == b"chunked")
  result = bytearray()
  while True:
    size_text, sep, rest = body.partition(b"\r\n")
    require(sep and re.fullmatch(rb"[0-9A-Fa-f]{1,8}", size_text) is not None)
    size = int(size_text, 16)
    if size == 0:
      require(rest == b"\r\n")
      return bytes(result)
    require(size <= MAX_RESPONSE - len(result) and len(rest) >= size + 2)
    require(rest[size:size + 2] == b"\r\n")
    result.extend(rest[:size])
    body = rest[size + 2:]


def parse_info(data):
  def unique(pairs):
    result = {}
    for key, value in pairs:
      require(key not in result)
      result[key] = value
    return result

  def invalid_constant(_value):
    raise Rejected()

  info = json.loads(decode_frame(data), object_pairs_hook=unique, parse_constant=invalid_constant)
  require(isinstance(info, dict))
  require(info.get("OSType") == "linux" and info.get("Architecture") in ("arm64", "aarch64"))
  version = info.get("ServerVersion")
  require(isinstance(version, str) and re.fullmatch(r"[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}", version))
  require(info.get("DockerRootDir") == "/var/lib/docker")
  return version


class Host:
  def environment(self):
    return os.environ

  def machine(self):
    return platform.machine()

  def free(self, path):
    info = os.statvfs(path)
    return info.f_bavail * info.f_frsize

  def snapshot(self):
    with check_stage("socket filesystem"):
      for path in ("/", "/run"):
        item = os.lstat(path)
        require(stat.S_ISDIR(item.st_mode) and item.st_uid == 0 and not item.st_mode & 0o022)
      item = os.lstat(SOCKET_PATH)
      require(stat.S_ISSOCK(item.st_mode) and item.st_uid == 0 and not item.st_mode & 0o002)
    with check_stage("daemon service"):
      result = subprocess.run(
        ["/usr/bin/systemctl", "show", "docker.service",
         "--property=MainPID,ActiveState,SubState"],
        check=True, capture_output=True, timeout=3,
        env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"},
      )
      pid = parse_service(result.stdout)
    with check_stage("daemon process"):
      require(os.readlink(f"/proc/{pid}/exe") == "/usr/bin/dockerd")
    with check_stage("daemon cgroup"):
      with open(f"/proc/{pid}/cgroup", "rb") as stream:
        cgroup = stream.read(4097)
      require(len(cgroup) <= 4096 and any(
        line.split(b":")[-1] == b"/system.slice/docker.service"
        for line in cgroup.splitlines()
      ))
    with check_stage("socket listener"):
      with open("/proc/net/unix", "rb") as stream:
        inode = listener_inode(stream.read(MAX_RESPONSE + 1))
    with check_stage("daemon listener fd"):
      fd_dir = f"/proc/{pid}/fd"
      links = []
      for name in os.listdir(fd_dir):
        try:
          links.append(os.readlink(f"{fd_dir}/{name}"))
        except FileNotFoundError:
          continue
      require(f"socket:[{inode}]" in links)
    return pid, item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_gid, inode

  def connect(self):
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(TIMEOUT)
    try:
      client.connect(SOCKET_PATH)
      return client
    except Exception:
      client.close()
      raise

  def peer(self, client):
    return struct.unpack("3i", client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12))

  def response(self, client):
    deadline = time.monotonic() + TIMEOUT
    client.settimeout(TIMEOUT)
    client.sendall(REQUEST)
    parts = bytearray()
    while True:
      remaining = deadline - time.monotonic()
      require(remaining > 0)
      client.settimeout(remaining)
      chunk = client.recv(min(65536, MAX_RESPONSE + 1 - len(parts)))
      if not chunk:
        return bytes(parts)
      parts.extend(chunk)
      require(len(parts) <= MAX_RESPONSE)


def collect(host):
  with check_stage("hosted environment"):
    env = host.environment()
    require(env.get("GITHUB_ACTIONS") == "true")
    require(env.get("RUNNER_ENVIRONMENT") == "github-hosted")
    require(env.get("RUNNER_ARCH") == "ARM64" and host.machine() == "aarch64")
    require(env.get("GITHUB_REPOSITORY") == "djensenius/chipviz")
    require(env.get("GITHUB_EVENT_NAME") == "pull_request")
    require(not any(key in env for key in ("DOCKER_HOST", "DOCKER_CONTEXT", "DOCKER_CONFIG")))
  with check_stage("host resources"):
    require(host.free("/") >= MIN_FREE)
  with check_stage("daemon resources"):
    require(host.free("/var/lib/docker") >= MIN_FREE)
  with check_stage("socket snapshot"):
    before = host.snapshot()
  with check_stage("socket connect"):
    with host.connect() as client:
      with check_stage("socket peer"):
        pid, uid, _gid = host.peer(client)
        require(uid == 0 and pid == before[0])
      with check_stage("socket recheck"):
        require(host.snapshot() == before)
      with check_stage("daemon response"):
        response = host.response(client)
      with check_stage("daemon metadata"):
        version = parse_info(response)
      with check_stage("socket recheck"):
        require(host.snapshot() == before)
  with check_stage("resource recheck"):
    require(host.free("/var/lib/docker") >= MIN_FREE)
  return version


def main():
  try:
    version = collect(Host())
  except Exception as error:
    print("host metadata: UNAVAILABLE")
    stage = getattr(error, "stage", None) if isinstance(error, StageError) else None
    print("failure stage: " + (stage if isinstance(stage, str) and stage in STAGES else "internal"))
    result = 1
  else:
    print("host metadata: COLLECTED")
    print("host architecture: arm64")
    print("daemon socket/service/peer: MATCHED")
    print("daemon architecture: arm64")
    print("daemon version: " + version)
    print("host and daemon-root free space: at least 5 GiB each")
    result = 0
  print("builder isolation: UNVERIFIED")
  print("mount isolation: UNVERIFIED")
  print("toolchain builds authorized: NO")
  return result


if __name__ == "__main__":
  raise SystemExit(main())
