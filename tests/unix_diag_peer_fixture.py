"""Test-only UNIX_DIAG_PEER calibration; not a production identity check."""

from __future__ import annotations

from contextlib import ExitStack
from dataclasses import dataclass
import math
import os
import re
import socket
import stat
import struct
import sys
import time


_U32_MAX = (1 << 32) - 1
NO_COOKIE = (_U32_MAX, _U32_MAX)
_NETLINK_SOCK_DIAG = 4
_SOCK_DIAG_BY_FAMILY = 20
_NLM_F_REQUEST = 1
_UDIAG_SHOW_PEER = 4
_UNIX_DIAG_PEER = 2
_UNIX_DIAG_SHUTDOWN = 6
_TCP_ESTABLISHED = 1
_HEADER = struct.Struct("=IHHII")
_REQUEST = struct.Struct("=BBHIIIII")
_DIAG = struct.Struct("=BBBBIII")
_ATTRIBUTE = struct.Struct("=HH")
_CREDENTIALS = struct.Struct("=iII")
_MAX_REPLY = 8192
_MAX_FDS = 64
_PING = b"fixture-ping"
_PONG = b"fixture-pong"


class EvidenceError(RuntimeError):
  """The fixture could not establish unambiguous evidence."""


@dataclass(frozen=True)
class Diagnostic:
  inode: int
  peer_inode: int
  cookie: tuple[int, int]


@dataclass(frozen=True)
class Calibration:
  inode: int
  peer_inode: int
  creator: tuple[int, int, int]
  child_fd: int
  cookie: tuple[int, int]


def _u32(value: int, *, nonzero: bool = False) -> bool:
  return type(value) is int and (1 if nonzero else 0) <= value <= _U32_MAX


def _remaining(deadline: float) -> float:
  if not isinstance(deadline, (int, float)) or not math.isfinite(deadline):
    raise EvidenceError("invalid deadline")
  remaining = deadline - time.monotonic()
  if remaining <= 0:
    raise EvidenceError("fixture timed out")
  return remaining


def decode_peer_reply(data: bytes, inode: int, sequence: int, port: int) -> Diagnostic:
  """Decode one exact-query reply, rejecting incomplete or ambiguous evidence."""
  if not _u32(inode, nonzero=True) or not _u32(sequence, nonzero=True) or not _u32(port, nonzero=True):
    raise EvidenceError("invalid query identity")
  if len(data) < _HEADER.size + _DIAG.size or len(data) > _MAX_REPLY:
    raise EvidenceError("truncated or oversized netlink reply")
  length, kind, flags, reply_sequence, reply_port = _HEADER.unpack_from(data)
  if length != len(data) or kind != _SOCK_DIAG_BY_FAMILY or flags != 0:
    raise EvidenceError("invalid netlink framing")
  if (reply_sequence, reply_port) != (sequence, port):
    raise EvidenceError("mismatched netlink identity")

  family, sock_type, state, padding, reply_inode, cookie_a, cookie_b = _DIAG.unpack_from(data, _HEADER.size)
  if (family, sock_type, state, padding, reply_inode) != (
      socket.AF_UNIX, socket.SOCK_STREAM, _TCP_ESTABLISHED, 0, inode):
    raise EvidenceError("mismatched socket record")
  cookie = (cookie_a, cookie_b)
  if cookie in ((0, 0), NO_COOKIE):
    raise EvidenceError("invalid socket cookie")

  offset = _HEADER.size + _DIAG.size
  seen = set()
  peer = None
  shutdown = None
  while offset < len(data):
    if len(data) - offset < _ATTRIBUTE.size:
      raise EvidenceError("truncated attribute header")
    attr_len, attr_type = _ATTRIBUTE.unpack_from(data, offset)
    padded_len = (attr_len + 3) & ~3
    if attr_len < _ATTRIBUTE.size or offset + padded_len > len(data):
      raise EvidenceError("invalid attribute length")
    if attr_type & 0xC000 or attr_type in seen:
      raise EvidenceError("flagged or duplicate attribute")
    seen.add(attr_type)
    payload = data[offset + _ATTRIBUTE.size:offset + attr_len]
    if attr_type == _UNIX_DIAG_PEER:
      if len(payload) != 4:
        raise EvidenceError("invalid peer attribute")
      peer = struct.unpack("=I", payload)[0]
    elif attr_type == _UNIX_DIAG_SHUTDOWN:
      if len(payload) != 1:
        raise EvidenceError("invalid shutdown attribute")
      shutdown = payload[0]
    offset += padded_len

  if not _u32(peer, nonzero=True) or peer == inode:
    raise EvidenceError("missing or invalid peer")
  if shutdown != 0:
    raise EvidenceError("missing or shut down socket")
  return Diagnostic(inode, peer, cookie)


def _open_diag_socket() -> socket.socket:
  if sys.platform != "linux":
    raise EvidenceError("netlink socket diagnostics require Linux")
  return socket.socket(socket.AF_NETLINK, socket.SOCK_RAW, _NETLINK_SOCK_DIAG)


def _query_peer(inode: int, cookie: tuple[int, int], sequence: int, deadline: float) -> Diagnostic:
  if (not _u32(inode, nonzero=True) or not _u32(sequence, nonzero=True)
      or not isinstance(cookie, tuple) or len(cookie) != 2 or not all(_u32(part) for part in cookie)):
    raise EvidenceError("invalid exact query")
  _remaining(deadline)
  try:
    with _open_diag_socket() as netlink:
      netlink.bind((0, 0))
      port, groups = netlink.getsockname()
      if not _u32(port, nonzero=True) or groups != 0:
        raise EvidenceError("invalid netlink port")
      request = _REQUEST.pack(
        socket.AF_UNIX, 0, 0, _U32_MAX, inode, _UDIAG_SHOW_PEER, *cookie,
      )
      request = _HEADER.pack(
        _HEADER.size + len(request), _SOCK_DIAG_BY_FAMILY, _NLM_F_REQUEST, sequence, port,
      ) + request
      netlink.settimeout(_remaining(deadline))
      if netlink.sendto(request, (0, 0)) != len(request):
        raise EvidenceError("short netlink request")
      netlink.settimeout(_remaining(deadline))
      data, ancillary, flags, sender = netlink.recvmsg(_MAX_REPLY)
      if ancillary or flags or sender != (0, 0):
        raise EvidenceError("truncated or untrusted netlink reply")
      result = decode_peer_reply(data, inode, sequence, port)
      _remaining(deadline)
      return result
  except TimeoutError as error:
    raise EvidenceError("netlink timed out") from error
  except OSError as error:
    raise EvidenceError("netlink I/O failed") from error


def _socket_identity(client: socket.socket) -> tuple[int, int, int]:
  try:
    fd = client.fileno()
    if fd < 0 or client.family != socket.AF_UNIX:
      raise EvidenceError("closed or non-UNIX client")
    info = os.fstat(fd)
    if (not stat.S_ISSOCK(info.st_mode)
        or client.getsockopt(socket.SOL_SOCKET, socket.SO_TYPE) != socket.SOCK_STREAM
        or not _u32(info.st_ino, nonzero=True)):
      raise EvidenceError("invalid client socket")
    return fd, info.st_dev, info.st_ino
  except OSError as error:
    raise EvidenceError("client socket unavailable") from error


def _peer_credentials(client: socket.socket) -> tuple[int, int, int]:
  if not hasattr(socket, "SO_PEERCRED"):
    raise EvidenceError("SO_PEERCRED requires Linux")
  try:
    data = client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, _CREDENTIALS.size)
  except OSError as error:
    raise EvidenceError("peer credentials unavailable") from error
  if len(data) != _CREDENTIALS.size:
    raise EvidenceError("truncated peer credentials")
  credentials = _CREDENTIALS.unpack(data)
  if credentials[0] <= 0:
    raise EvidenceError("invalid peer credentials")
  return credentials


def _child_running(child, deadline: float) -> None:
  _remaining(deadline)
  if type(child.pid) is not int or child.pid <= 0 or child.poll() is not None:
    raise EvidenceError("fixture child is not running")
  _remaining(deadline)


def _read_child_fds(child, deadline: float) -> dict[int, str]:
  """Read only the known fixture child's FD links, failing on any race."""
  _child_running(child, deadline)
  directory = f"/proc/{child.pid}/fd"
  try:
    names = os.listdir(directory)
    if len(names) > _MAX_FDS:
      raise EvidenceError("too many fixture child FDs")
    links = {}
    for name in sorted(names):
      _remaining(deadline)
      if not re.fullmatch(r"0|[1-9][0-9]{0,9}", name):
        raise EvidenceError("invalid fixture FD name")
      fd = int(name)
      if fd in links:
        raise EvidenceError("duplicate fixture FD")
      links[fd] = os.readlink(f"{directory}/{name}")
  except OSError as error:
    raise EvidenceError("fixture FD snapshot failed") from error
  _child_running(child, deadline)
  return links


def _check_owner(links: dict[int, str], child_fd: int, peer_inode: int) -> None:
  if type(child_fd) is not int or child_fd < 0 or not _u32(peer_inode, nonzero=True):
    raise EvidenceError("invalid owner evidence")
  matches = []
  for fd, target in links.items():
    if type(fd) is not int or fd < 0 or not isinstance(target, str):
      raise EvidenceError("invalid fixture FD evidence")
    if target.startswith("socket:"):
      match = re.fullmatch(r"socket:\[([1-9][0-9]{0,19})\]", target)
      if not match:
        raise EvidenceError("malformed fixture socket link")
      if int(match.group(1)) == peer_inode:
        matches.append(fd)
  if matches != [child_fd]:
    raise EvidenceError("missing, duplicate, or mismatched peer FD")


def _hold_child(control: socket.socket, child_fd: int, deadline: float) -> None:
  send_line(control, b"HOLD", deadline)
  if receive_line(control, deadline) != f"HELD {child_fd}".encode("ascii"):
    raise EvidenceError("fixture observation barrier rejected")


def calibrate(client: socket.socket, child, child_fd: int, expected_creator: tuple[int, int, int],
              control: socket.socket, deadline: float) -> Calibration:
  """Recheck evidence while the cooperative child stays held through return."""
  _child_running(child, deadline)
  before = _socket_identity(client)
  creator = _peer_credentials(client)
  if creator != expected_creator:
    raise EvidenceError("unexpected listener creator credentials")
  first = _query_peer(before[2], NO_COOKIE, 1, deadline)
  _check_owner(_read_child_fds(child, deadline), child_fd, first.peer_inode)
  second = _query_peer(before[2], first.cookie, 2, deadline)
  if second != first:
    raise EvidenceError("peer or socket cookie changed")
  _check_owner(_read_child_fds(child, deadline), child_fd, second.peer_inode)
  if _socket_identity(client) != before or _peer_credentials(client) != creator:
    raise EvidenceError("client socket or credentials changed")
  _child_running(child, deadline)
  _hold_child(control, child_fd, deadline)
  final = _query_peer(before[2], first.cookie, 3, deadline)
  if final != first:
    raise EvidenceError("peer or socket cookie changed")
  _check_owner(_read_child_fds(child, deadline), child_fd, final.peer_inode)
  if _socket_identity(client) != before or _peer_credentials(client) != creator:
    raise EvidenceError("client socket or credentials changed")
  _child_running(child, deadline)
  return Calibration(before[2], first.peer_inode, creator, child_fd, first.cookie)


def send_bytes(sock: socket.socket, data: bytes, deadline: float) -> None:
  try:
    sock.settimeout(_remaining(deadline))
    sock.sendall(data)
    _remaining(deadline)
  except TimeoutError as error:
    raise EvidenceError("fixture I/O timed out") from error
  except OSError as error:
    raise EvidenceError("fixture I/O failed") from error


def receive_bytes(sock: socket.socket, size: int, deadline: float) -> bytes:
  if type(size) is not int or not 0 < size <= 128:
    raise EvidenceError("invalid fixture read size")
  result = bytearray()
  while len(result) < size:
    try:
      sock.settimeout(_remaining(deadline))
      chunk = sock.recv(size - len(result))
    except TimeoutError as error:
      raise EvidenceError("fixture I/O timed out") from error
    except OSError as error:
      raise EvidenceError("fixture I/O failed") from error
    if not chunk:
      raise EvidenceError("fixture channel closed")
    result.extend(chunk)
  _remaining(deadline)
  return bytes(result)


def send_line(sock: socket.socket, line: bytes, deadline: float) -> None:
  if not line or len(line) > 64 or b"\n" in line:
    raise EvidenceError("invalid fixture line")
  send_bytes(sock, line + b"\n", deadline)


def receive_line(sock: socket.socket, deadline: float) -> bytes:
  line = bytearray()
  for _ in range(65):
    char = receive_bytes(sock, 1, deadline)
    if char == b"\n":
      if not line:
        raise EvidenceError("invalid fixture line")
      return bytes(line)
    line.extend(char)
  raise EvidenceError("invalid fixture line")


def _fixture_child(mode: str, path: str, control_fd: int, listener_fd: int) -> None:
  deadline = time.monotonic() + 20
  with socket.socket(fileno=control_fd) as control:
    if mode == "activated":
      listener = socket.socket(fileno=listener_fd)
    elif mode == "direct":
      listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      listener.bind(path)
      listener.listen(1)
    else:
      raise EvidenceError("invalid fixture mode")
    with listener, ExitStack() as stack:
      listener.settimeout(_remaining(deadline))
      send_line(control, b"READY", deadline)
      accepted, _ = listener.accept()
      stack.enter_context(accepted)
      accepted_fd = accepted.fileno()
      active = accepted
      held = False
      replaced = False
      send_line(control, f"ACCEPT {accepted_fd}".encode("ascii"), deadline)
      if receive_bytes(active, len(_PING), deadline) != _PING:
        raise EvidenceError("unexpected fixture challenge")
      send_bytes(active, _PONG, deadline)
      send_line(control, b"EXCHANGED", deadline)
      while True:
        command = receive_line(control, deadline)
        if command == b"RELEASE":
          return
        if command == b"HOLD" and not held:
          held = True
          send_line(control, f"HELD {accepted_fd}".encode("ascii"), deadline)
        elif command == b"REPLACE" and held:
          send_line(control, b"BUSY", deadline)
        elif command == b"REPLACE" and not replaced:
          active = stack.enter_context(accepted.dup())
          accepted.close()
          replacement, spare = socket.socketpair()
          stack.enter_context(replacement)
          stack.enter_context(spare)
          if replacement.fileno() != accepted_fd:
            raise EvidenceError("fixture did not reuse the accepted FD")
          replaced = True
          send_line(control, f"REPLACED {accepted_fd} {active.fileno()}".encode("ascii"), deadline)
        elif command == b"CHECK":
          if receive_bytes(active, len(_PING), deadline) != _PING:
            raise EvidenceError("unexpected fixture challenge")
          send_bytes(active, _PONG, deadline)
          send_line(control, b"CHECKED", deadline)
        else:
          raise EvidenceError("unexpected fixture command")


if __name__ == "__main__":
  if len(sys.argv) != 6 or sys.argv[1] != "child":
    raise SystemExit("fixture child invocation only")
  _fixture_child(sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]))
