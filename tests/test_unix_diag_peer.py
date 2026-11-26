from contextlib import ExitStack, contextmanager
import os
from pathlib import Path
import re
import socket
import stat
import struct
import subprocess
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest import mock

import unix_diag_peer_fixture as proof


ROOT = Path(__file__).resolve().parents[1]
INODE = 101
PEER = 202
COOKIE = (33, 44)
SEQUENCE = 7
PORT = 9001


def attribute(kind, payload):
  length = 4 + len(payload)
  return struct.pack("=HH", length, kind) + payload + bytes((-length) % 4)


def reply(*, inode=INODE, peer=PEER, cookie=COOKIE, sequence=SEQUENCE, port=PORT,
          family=socket.AF_UNIX, sock_type=socket.SOCK_STREAM, state=1, padding=0,
          kind=20, flags=0, attributes=None):
  if attributes is None:
    attributes = attribute(2, struct.pack("=I", peer)) + attribute(6, b"\0")
  body = struct.pack("=BBBBIII", family, sock_type, state, padding, inode, *cookie) + attributes
  return struct.pack("=IHHII", 16 + len(body), kind, flags, sequence, port) + body


class DecoderTests(unittest.TestCase):
  def decode(self, data):
    return proof.decode_peer_reply(data, INODE, SEQUENCE, PORT)

  def test_valid_reply_decodes_peer_cookie_and_aligned_unknown_attribute(self):
    attrs = (attribute(2, struct.pack("=I", PEER))
             + attribute(99, b"abc") + attribute(6, b"\0"))
    self.assertEqual(self.decode(reply(attributes=attrs)), proof.Diagnostic(INODE, PEER, COOKIE))

  def test_rejects_bad_headers_and_mismatched_socket_records(self):
    good = reply()
    cases = {
      "empty": b"",
      "short header": good[:15],
      "short record": good[:31],
      "truncated": good[:-1],
      "trailing bytes": good + b"\0",
      "duplicate records": good + good,
      "error message": reply(kind=2),
      "multipart": reply(flags=2),
      "sequence": reply(sequence=SEQUENCE + 1),
      "port": reply(port=PORT + 1),
      "inode": reply(inode=INODE + 1),
      "family": reply(family=socket.AF_INET),
      "type": reply(sock_type=socket.SOCK_DGRAM),
      "state": reply(state=10),
      "padding": reply(padding=1),
      "zero cookie": reply(cookie=(0, 0)),
      "wildcard cookie": reply(cookie=proof.NO_COOKIE),
    }
    for name, data in cases.items():
      with self.subTest(name=name), self.assertRaises(proof.EvidenceError):
        self.decode(data)

  def test_rejects_malformed_and_truncated_attributes(self):
    shutdown = attribute(6, b"\0")
    malformed = {
      "partial header": b"\x04\x00\x02",
      "length below header": struct.pack("=HH", 3, 2),
      "length beyond record": struct.pack("=HHI", 12, 2, PEER),
      "missing alignment": struct.pack("=HH", 5, 6) + b"\0",
      "short peer": attribute(2, b"\1\2\3") + shutdown,
      "long peer": attribute(2, struct.pack("=II", PEER, 1)) + shutdown,
      "short shutdown": attribute(2, struct.pack("=I", PEER)) + attribute(6, b""),
      "flagged attribute": attribute(0x8002, struct.pack("=I", PEER)) + shutdown,
      "trailing fragment": attribute(2, struct.pack("=I", PEER)) + shutdown + b"\0\0",
    }
    for name, attributes in malformed.items():
      with self.subTest(name=name), self.assertRaises(proof.EvidenceError):
        self.decode(reply(attributes=attributes))

  def test_rejects_missing_duplicate_or_closed_peer_evidence(self):
    peer = attribute(2, struct.pack("=I", PEER))
    shutdown = attribute(6, b"\0")
    cases = {
      "missing peer": shutdown,
      "zero peer": attribute(2, struct.pack("=I", 0)) + shutdown,
      "self peer": attribute(2, struct.pack("=I", INODE)) + shutdown,
      "duplicate identical peer": peer + peer + shutdown,
      "duplicate different peer": peer + attribute(2, struct.pack("=I", PEER + 1)) + shutdown,
      "missing shutdown": peer,
      "duplicate shutdown": peer + shutdown + shutdown,
      "shut down socket": peer + attribute(6, b"\1"),
      "duplicate unknown": peer + attribute(99, b"") + attribute(99, b"") + shutdown,
    }
    for name, attributes in cases.items():
      with self.subTest(name=name), self.assertRaises(proof.EvidenceError):
        self.decode(reply(attributes=attributes))

  def test_rejects_invalid_expected_query_identity(self):
    for inode, sequence, port in ((0, SEQUENCE, PORT), (INODE, 0, PORT),
                                  (INODE, SEQUENCE, 0), (True, SEQUENCE, PORT),
                                  (1 << 32, SEQUENCE, PORT)):
      with self.subTest(inode=inode, sequence=sequence, port=port):
        with self.assertRaises(proof.EvidenceError):
          proof.decode_peer_reply(reply(), inode, sequence, port)


class FakeNetlink:
  def __init__(self, *, data=None, sender=(0, 0), flags=0, ancillary=None, error=None,
               sent_length=None):
    self.data = reply() if data is None else data
    self.sender = sender
    self.flags = flags
    self.ancillary = [] if ancillary is None else ancillary
    self.error = error
    self.sent_length = sent_length
    self.sent = None
    self.timeouts = []

  def __enter__(self):
    return self

  def __exit__(self, *args):
    return False

  def bind(self, address):
    self.bound = address

  def getsockname(self):
    return PORT, 0

  def settimeout(self, timeout):
    self.timeouts.append(timeout)

  def sendto(self, data, address):
    self.sent = data, address
    return len(data) if self.sent_length is None else self.sent_length

  def recvmsg(self, size):
    self.size = size
    if self.error is not None:
      raise self.error
    return self.data, self.ancillary, self.flags, self.sender


class NetlinkQueryTests(unittest.TestCase):
  def query(self, fake, cookie=proof.NO_COOKIE):
    with mock.patch.object(proof, "_open_diag_socket", return_value=fake):
      return proof._query_peer(INODE, cookie, SEQUENCE, time.monotonic() + 5)

  def test_request_is_exact_and_targets_only_the_fixture_inode(self):
    fake = FakeNetlink()
    self.assertEqual(self.query(fake, COOKIE), proof.Diagnostic(INODE, PEER, COOKIE))
    data, destination = fake.sent
    self.assertEqual(fake.bound, (0, 0))
    self.assertEqual(destination, (0, 0))
    self.assertEqual(struct.unpack_from("=IHHII", data), (40, 20, 1, SEQUENCE, PORT))
    self.assertEqual(struct.unpack_from("=BBHIIIII", data, 16),
                     (socket.AF_UNIX, 0, 0, 0xFFFFFFFF, INODE, 4, *COOKIE))
    self.assertEqual(len(data), 40)
    self.assertEqual(len(fake.timeouts), 2)
    self.assertTrue(all(0 < timeout <= 5 for timeout in fake.timeouts))
    self.assertEqual(fake.size, 8192)

  def test_rejects_untrusted_truncated_or_timed_out_replies(self):
    cases = (
      FakeNetlink(sender=(1, 0)),
      FakeNetlink(sender=(0, 1)),
      FakeNetlink(flags=socket.MSG_TRUNC),
      FakeNetlink(ancillary=[(1, 2, b"x")]),
      FakeNetlink(error=TimeoutError()),
      FakeNetlink(error=OSError("fixture netlink error")),
      FakeNetlink(sent_length=39),
    )
    for fake in cases:
      with self.subTest(sender=fake.sender, flags=fake.flags, error=fake.error):
        with self.assertRaises(proof.EvidenceError):
          self.query(fake)

  def test_invalid_queries_and_deadlines_never_open_netlink(self):
    deadline = time.monotonic() + 5
    cases = ((0, proof.NO_COOKIE, SEQUENCE, deadline),
             (1 << 32, proof.NO_COOKIE, SEQUENCE, deadline),
             (INODE, (1,), SEQUENCE, deadline),
             (INODE, (1, -1), SEQUENCE, deadline),
             (INODE, proof.NO_COOKIE, 0, deadline),
             (INODE, proof.NO_COOKIE, SEQUENCE, time.monotonic() - 1),
             (INODE, proof.NO_COOKIE, SEQUENCE, float("inf")))
    with mock.patch.object(proof, "_open_diag_socket") as opened:
      for args in cases:
        with self.subTest(args=args), self.assertRaises(proof.EvidenceError):
          proof._query_peer(*args)
      opened.assert_not_called()


class CredentialTests(unittest.TestCase):
  def test_decodes_known_credentials_and_rejects_bad_records(self):
    client = mock.Mock()
    with mock.patch.object(proof.socket, "SO_PEERCRED", 17, create=True):
      client.getsockopt.return_value = struct.pack("=iII", 123, 501, 20)
      self.assertEqual(proof._peer_credentials(client), (123, 501, 20))
      client.getsockopt.assert_called_once_with(socket.SOL_SOCKET, 17, 12)
      for data in (b"", b"\0" * 11, struct.pack("=iII", 0, 501, 20),
                   struct.pack("=iII", -1, 501, 20)):
        client.getsockopt.return_value = data
        with self.subTest(data=data), self.assertRaises(proof.EvidenceError):
          proof._peer_credentials(client)
      client.getsockopt.side_effect = OSError("fixture credential error")
      with self.assertRaises(proof.EvidenceError):
        proof._peer_credentials(client)


class OwnershipTests(unittest.TestCase):
  def test_requires_exactly_one_matching_fd_at_the_reported_number(self):
    links = {0: "/dev/null", 4: "socket:[55]", 8: f"socket:[{PEER}]"}
    proof._check_owner(links, 8, PEER)
    cases = (
      (links, 4, PEER),
      ({0: "/dev/null", 4: "socket:[55]"}, 8, PEER),
      ({8: f"socket:[{PEER}]", 9: f"socket:[{PEER}]"}, 8, PEER),
      ({8: "socket:[203]"}, 8, PEER),
      ({8: "socket:[0202]"}, 8, PEER),
      ({8: "socket:[202", 9: f"socket:[{PEER}]"}, 9, PEER),
      ({8: None}, 8, PEER),
      (links, -1, PEER),
      (links, 8, 0),
    )
    for snapshot, fd, peer in cases:
      with self.subTest(snapshot=snapshot, fd=fd, peer=peer):
        with self.assertRaises(proof.EvidenceError):
          proof._check_owner(snapshot, fd, peer)

  def test_snapshot_reads_only_the_known_child_fd_directory(self):
    child = SimpleNamespace(pid=4321, poll=mock.Mock(return_value=None))
    directory = "/proc/4321/fd"
    targets = {f"{directory}/0": "/dev/null",
               f"{directory}/8": f"socket:[{PEER}]"}
    with mock.patch.object(proof.os, "listdir", return_value=["8", "0"]) as listing, \
         mock.patch.object(proof.os, "readlink", side_effect=targets.__getitem__) as readlink:
      links = proof._read_child_fds(child, time.monotonic() + 5)
    self.assertEqual(links, {0: "/dev/null", 8: f"socket:[{PEER}]"})
    listing.assert_called_once_with(directory)
    self.assertEqual(readlink.call_args_list,
                     [mock.call(f"{directory}/0"), mock.call(f"{directory}/8")])

  def test_disappearing_fd_missing_proc_and_child_exit_fail_closed(self):
    child = SimpleNamespace(pid=4321, poll=mock.Mock(return_value=None))
    with mock.patch.object(proof.os, "listdir", return_value=["8"]), \
         mock.patch.object(proof.os, "readlink", side_effect=FileNotFoundError):
      with self.assertRaises(proof.EvidenceError):
        proof._read_child_fds(child, time.monotonic() + 5)
    with mock.patch.object(proof.os, "listdir", side_effect=FileNotFoundError):
      with self.assertRaises(proof.EvidenceError):
        proof._read_child_fds(child, time.monotonic() + 5)
    child.poll.side_effect = [None, 0]
    with mock.patch.object(proof.os, "listdir", return_value=["8"]), \
         mock.patch.object(proof.os, "readlink", return_value=f"socket:[{PEER}]"):
      with self.assertRaises(proof.EvidenceError):
        proof._read_child_fds(child, time.monotonic() + 5)

  def test_rejects_invalid_or_unbounded_fd_list(self):
    child = SimpleNamespace(pid=4321, poll=mock.Mock(return_value=None))
    for names in (["01"], ["-1"], ["1", "1"], [str(i) for i in range(65)]):
      with self.subTest(names=names), \
           mock.patch.object(proof.os, "listdir", return_value=names), \
           mock.patch.object(proof.os, "readlink", return_value="/dev/null"):
        with self.assertRaises(proof.EvidenceError):
          proof._read_child_fds(child, time.monotonic() + 5)


class CalibrationTests(unittest.TestCase):
  def setUp(self):
    self.real_identity = proof._socket_identity
    stack = ExitStack()
    self.addCleanup(stack.close)
    self.identity = stack.enter_context(mock.patch.object(proof, "_socket_identity"))
    self.credentials = stack.enter_context(mock.patch.object(proof, "_peer_credentials"))
    self.query = stack.enter_context(mock.patch.object(proof, "_query_peer"))
    self.fds = stack.enter_context(mock.patch.object(proof, "_read_child_fds"))
    self.hold = stack.enter_context(mock.patch.object(proof, "_hold_child"))
    self.client = mock.Mock()
    self.pinned = mock.Mock()
    self.client.dup.return_value = self.pinned
    self.control = object()
    self.child = SimpleNamespace(pid=456, poll=mock.Mock(return_value=None))
    self.creator = (123, 501, 20)
    self.identity.side_effect = [(4, 9, INODE)] * 3
    self.credentials.side_effect = [self.creator] * 3
    self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE)] * 3
    self.fds.side_effect = [{8: f"socket:[{PEER}]"}] * 3

  def calibrate(self):
    with proof.calibrate(self.client, self.child, 8, self.creator, self.control, time.monotonic() + 5) as evidence:
      self.assertIs(evidence.pinned_client, self.pinned)
      return evidence.inode, evidence.peer_inode, evidence.creator, evidence.child_fd, evidence.cookie

  def test_stable_evidence_rechecks_the_cookie_owner_and_credentials(self):
    self.assertEqual(self.calibrate(), (INODE, PEER, self.creator, 8, COOKIE))
    self.client.dup.assert_called_once_with()
    self.pinned.close.assert_called_once_with()
    self.assertEqual(self.identity.call_args_list, [mock.call(self.pinned)] * 3)
    self.assertEqual(self.credentials.call_args_list, [mock.call(self.pinned)] * 3)
    self.assertEqual(self.query.call_count, 3)
    self.assertEqual(self.query.call_args_list[0].args[:3], (INODE, proof.NO_COOKIE, 1))
    self.assertEqual(self.query.call_args_list[1].args[:3], (INODE, COOKIE, 2))
    self.assertEqual(self.query.call_args_list[2].args[:3], (INODE, COOKIE, 3))
    self.assertEqual(self.fds.call_count, 3)
    self.assertEqual(self.credentials.call_count, 3)
    self.hold.assert_called_once_with(self.control, 8, mock.ANY)

  def test_wrong_creator_never_queries_the_socket(self):
    self.credentials.side_effect = [(999, 501, 20)]
    with self.assertRaisesRegex(proof.EvidenceError, "creator"):
      self.calibrate()
    self.query.assert_not_called()
    self.pinned.close.assert_called_once_with()

  def test_changed_credentials_peer_cookie_and_owner_are_rejected(self):
    self.credentials.side_effect = [self.creator, (123, 502, 20)]
    with self.assertRaisesRegex(proof.EvidenceError, "credentials changed"):
      self.calibrate()

    self.credentials.side_effect = [self.creator, self.creator]
    self.identity.side_effect = [(4, 9, INODE)] * 2
    self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE),
                              proof.Diagnostic(INODE, PEER + 1, COOKIE)]
    self.fds.side_effect = [{8: f"socket:[{PEER}]"}] * 2
    with self.assertRaisesRegex(proof.EvidenceError, "peer or socket cookie changed"):
      self.calibrate()

    self.credentials.side_effect = [self.creator, self.creator]
    self.identity.side_effect = [(4, 9, INODE)] * 2
    self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE),
                              proof.Diagnostic(INODE, PEER, (33, 45))]
    self.fds.side_effect = [{8: f"socket:[{PEER}]"}] * 2
    with self.assertRaisesRegex(proof.EvidenceError, "peer or socket cookie changed"):
      self.calibrate()

    self.credentials.side_effect = [self.creator, self.creator]
    self.identity.side_effect = [(4, 9, INODE)] * 2
    self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE)] * 2
    self.fds.side_effect = [{8: f"socket:[{PEER}]"}, {8: "socket:[203]"}]
    with self.assertRaisesRegex(proof.EvidenceError, "peer FD"):
      self.calibrate()

  def test_missing_or_duplicate_owner_stops_before_second_query(self):
    for links in ({8: "socket:[203]"}, {8: f"socket:[{PEER}]", 9: f"socket:[{PEER}]"}):
      self.identity.side_effect = [(4, 9, INODE)] * 2
      self.credentials.side_effect = [self.creator, self.creator]
      self.query.reset_mock()
      self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE)] * 2
      self.fds.side_effect = [links]
      with self.subTest(links=links), self.assertRaisesRegex(proof.EvidenceError, "peer FD"):
        self.calibrate()
      self.assertEqual(self.query.call_count, 1)

  def test_child_exit_and_expired_deadline_are_rejected(self):
    self.child.poll.side_effect = [None, 0]
    with self.assertRaisesRegex(proof.EvidenceError, "child"):
      self.calibrate()
    self.query.reset_mock()
    with self.assertRaisesRegex(proof.EvidenceError, "timed out"):
      with proof.calibrate(self.client, self.child, 8, self.creator, self.control, time.monotonic() - 1):
        self.fail("expired calibration succeeded")
    self.query.assert_not_called()

  def test_barrier_failure_stops_before_final_observation(self):
    self.hold.side_effect = proof.EvidenceError("fixture observation barrier rejected")
    with self.assertRaisesRegex(proof.EvidenceError, "barrier"):
      self.calibrate()
    self.assertEqual(self.query.call_count, 2)
    self.assertEqual(self.fds.call_count, 2)

  def test_held_final_cookie_mismatch_is_rejected(self):
    self.query.side_effect = [proof.Diagnostic(INODE, PEER, COOKIE)] * 2 + [
      proof.Diagnostic(INODE, PEER, (33, 45)),
    ]
    with self.assertRaisesRegex(proof.EvidenceError, "peer or socket cookie changed"):
      self.calibrate()
    self.hold.assert_called_once()
    self.assertEqual(self.fds.call_count, 2)

  def test_real_pinned_client_close_and_fd_replacement_are_rejected(self):
    for replacement in (False, True):
      with self.subTest(replacement=replacement), ExitStack() as stack:
        client, peer = socket.socketpair()
        other, other_peer = socket.socketpair()
        for sock in (client, peer, other, other_peer):
          stack.enter_context(sock)
        original_inode = os.fstat(client.fileno()).st_ino
        self.assertNotEqual(original_inode, os.fstat(other.fileno()).st_ino)
        pinned = []

        def identity(sock):
          if not pinned:
            pinned.append(sock)
          return self.real_identity(sock)

        self.identity.side_effect = identity
        self.credentials.side_effect = [self.creator, self.creator]
        self.fds.side_effect = [{8: f"socket:[{PEER}]"}] * 2

        def race(inode, cookie, sequence, deadline):
          if sequence == 2:
            if replacement:
              os.dup2(other.fileno(), pinned[0].fileno())
            else:
              pinned[0].close()
          return proof.Diagnostic(inode, PEER, COOKIE)

        self.query.side_effect = race
        reason = "client socket or credentials changed" if replacement else "closed or non-UNIX client"
        with self.assertRaisesRegex(proof.EvidenceError, reason):
          with proof.calibrate(client, self.child, 8, self.creator, self.control, time.monotonic() + 5):
            self.fail("changed pinned socket was accepted")
        self.assertEqual(pinned[0].fileno(), -1)
        self.assertEqual(os.fstat(client.fileno()).st_ino, original_inode)

  def test_closed_client_cannot_be_pinned(self):
    client, peer = socket.socketpair()
    with client, peer:
      client.close()
      with self.assertRaisesRegex(proof.EvidenceError, "client socket unavailable"):
        with proof.calibrate(client, self.child, 8, self.creator, self.control, time.monotonic() + 5):
          self.fail("closed client was accepted")
    self.query.assert_not_called()

  def test_pin_closes_if_consumer_raises(self):
    client, peer = socket.socketpair()
    with client, peer:
      client.settimeout(1)
      peer.settimeout(1)
      self.identity.side_effect = self.real_identity
      self.query.side_effect = lambda inode, cookie, sequence, deadline: proof.Diagnostic(inode, PEER, COOKIE)
      pinned = None
      with self.assertRaisesRegex(RuntimeError, "consumer failed"):
        with proof.calibrate(client, self.child, 8, self.creator, self.control, time.monotonic() + 5) as evidence:
          pinned = evidence.pinned_client
          self.assertNotEqual(pinned.fileno(), client.fileno())
          raise RuntimeError("consumer failed")
      self.assertEqual(pinned.fileno(), -1)
      client.sendall(b"x")
      self.assertEqual(peer.recv(1), b"x")


class FixtureIOTests(unittest.TestCase):
  def test_timeout_and_closed_channel_are_not_accepted(self):
    left, right = socket.socketpair()
    with left, right:
      with self.assertRaisesRegex(proof.EvidenceError, "timed out"):
        proof.receive_bytes(left, 1, time.monotonic() + 0.03)
      right.close()
      with self.assertRaisesRegex(proof.EvidenceError, "closed"):
        proof.receive_line(left, time.monotonic() + 1)

  def test_line_framing_has_a_bounded_length(self):
    left, right = socket.socketpair()
    with left, right:
      proof.send_line(right, b"READY", time.monotonic() + 1)
      self.assertEqual(proof.receive_line(left, time.monotonic() + 1), b"READY")
      right.sendall(b"x" * 65 + b"\n")
      with self.assertRaisesRegex(proof.EvidenceError, "line"):
        proof.receive_line(left, time.monotonic() + 1)
      with self.assertRaisesRegex(proof.EvidenceError, "line"):
        proof.receive_line(left, time.monotonic() + 1)

  def test_partial_reads_do_not_extend_the_absolute_deadline(self):
    slow = mock.Mock()
    slow.recv.return_value = b"x"
    with mock.patch.object(proof.time, "monotonic", side_effect=[100.0, 101.1]):
      with self.assertRaisesRegex(proof.EvidenceError, "timed out"):
        proof.receive_bytes(slow, 2, 101.0)
    slow.recv.assert_called_once_with(2)

  def test_observation_barrier_rejects_bad_ack_and_timeout(self):
    left, right = socket.socketpair()
    with left, right:
      right.sendall(b"HELD 9\n")
      with self.assertRaisesRegex(proof.EvidenceError, "barrier"):
        proof._hold_child(left, 8, time.monotonic() + 1)
      self.assertEqual(proof.receive_line(right, time.monotonic() + 1), b"HOLD")
    left, right = socket.socketpair()
    with left, right:
      with self.assertRaisesRegex(proof.EvidenceError, "timed out"):
        proof._hold_child(left, 8, time.monotonic() + 0.03)


def check_fixture_path(path):
  encoded = os.fsencode(path)
  # Darwin's 104-byte sun_path includes the terminating NUL.
  if not encoded or b"\0" in encoded or len(encoded) >= 104:
    raise proof.EvidenceError("fixture socket path is too long or invalid")


def stop_fixture_child(child, control):
  if child.poll() is None:
    try:
      proof.send_line(control, b"RELEASE", time.monotonic() + 1)
    except proof.EvidenceError:
      pass
    try:
      child.wait(timeout=2)
    except subprocess.TimeoutExpired:
      child.terminate()
      try:
        child.wait(timeout=2)
      except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=2)


@contextmanager
def owned_fixture(mode):
  deadline = time.monotonic() + 15
  with ExitStack() as stack:
    directory = stack.enter_context(tempfile.TemporaryDirectory(prefix="cvz-", dir="/tmp"))
    path = str(Path(directory) / "s")
    check_fixture_path(path)
    control, child_control = socket.socketpair()
    stack.enter_context(control)
    stack.enter_context(child_control)
    passed_fds = [child_control.fileno()]
    listener_fd = -1
    if mode == "activated":
      listener = stack.enter_context(socket.socket(socket.AF_UNIX, socket.SOCK_STREAM))
      listener.bind(path)
      listener.listen(1)
      listener_fd = listener.fileno()
      passed_fds.append(listener_fd)
    child = subprocess.Popen(
      [sys.executable, proof.__file__, "child", mode, path, str(child_control.fileno()), str(listener_fd)],
      cwd=ROOT, pass_fds=tuple(passed_fds), stdin=subprocess.DEVNULL,
      stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, env={"PYTHONDONTWRITEBYTECODE": "1"},
    )
    stack.callback(child.stderr.close)
    stack.callback(stop_fixture_child, child, control)
    child_control.close()
    if proof.receive_line(control, deadline) != b"READY":
      raise proof.EvidenceError("fixture listener was not ready")
    client = stack.enter_context(socket.socket(socket.AF_UNIX, socket.SOCK_STREAM))
    client.settimeout(proof._remaining(deadline))
    client.connect(path)
    message = proof.receive_line(control, deadline)
    match = re.fullmatch(rb"ACCEPT (0|[1-9][0-9]*)", message)
    if not match:
      raise proof.EvidenceError("invalid accepted FD report")
    child_fd = int(match.group(1))
    proof.send_bytes(client, proof._PING, deadline)
    if proof.receive_bytes(client, len(proof._PONG), deadline) != proof._PONG:
      raise proof.EvidenceError("fixture challenge failed")
    if proof.receive_line(control, deadline) != b"EXCHANGED":
      raise proof.EvidenceError("fixture exchange was not confirmed")
    creator_pid = os.getpid() if mode == "activated" else child.pid
    creator = (creator_pid, os.geteuid(), os.getegid())

    def finish():
      proof.send_line(control, b"RELEASE", deadline)
      child.wait(timeout=proof._remaining(deadline))
      if child.returncode != 0:
        raise AssertionError(f"fixture child failed: {child.stderr.read(4096)!r}")

    yield SimpleNamespace(child=child, control=control, client=client, child_fd=child_fd, path=path,
                          creator=creator, deadline=deadline, finish=finish)


def replace_fixture_fd(fixture, deadline):
  proof.send_line(fixture.control, b"REPLACE", deadline)
  message = proof.receive_line(fixture.control, deadline)
  match = re.fullmatch(rb"REPLACED (0|[1-9][0-9]*) (0|[1-9][0-9]*)", message)
  if not match or int(match.group(1)) != fixture.child_fd:
    raise AssertionError(f"invalid replacement report: {message!r}")
  duplicate_fd = int(match.group(2))
  if duplicate_fd == fixture.child_fd:
    raise AssertionError("replacement did not retain a distinct duplicate")
  return duplicate_fd


def exchange_fixture_peer(fixture, deadline, client=None):
  if client is None:
    client = fixture.client
  proof.send_line(fixture.control, b"CHECK", deadline)
  proof.send_bytes(client, proof._PING, deadline)
  return (proof.receive_bytes(client, len(proof._PONG), deadline),
          proof.receive_line(fixture.control, deadline))


@unittest.skipUnless(os.name == "posix", "Unix socket fixture requires POSIX")
class PortableFixtureTests(unittest.TestCase):
  def test_inherited_and_direct_listener_lifecycles(self):
    for mode in ("activated", "direct"):
      with self.subTest(mode=mode), owned_fixture(mode) as fixture:
        self.assertIsNone(fixture.child.poll())
        self.assertNotEqual(fixture.child.pid, os.getpid())
        self.assertGreater(fixture.child_fd, 2)
        self.assertEqual(Path(fixture.path).parent.parent, Path("/tmp"))
        self.assertLess(len(os.fsencode(fixture.path)), 104)
        self.assertEqual(stat.S_IMODE(os.stat(Path(fixture.path).parent).st_mode), 0o700)
        if mode == "direct":
          replace_fixture_fd(fixture, fixture.deadline)
          self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline), (proof._PONG, b"CHECKED"))
        fixture.finish()
        self.assertEqual(fixture.child.returncode, 0)

  def test_socket_path_byte_limit_in_a_private_short_directory(self):
    with tempfile.TemporaryDirectory(prefix="cvz-", dir="/tmp") as directory:
      self.assertEqual(stat.S_IMODE(os.stat(directory).st_mode), 0o700)
      prefix = directory + "/"
      count = 103 - len(os.fsencode(prefix))
      self.assertGreater(count, 0)
      path = prefix + "s" * count
      self.assertEqual(len(os.fsencode(path)), 103)
      check_fixture_path(path)
      with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as listener:
        listener.bind(path)
        listener.listen(1)
        self.assertTrue(stat.S_ISSOCK(os.stat(path).st_mode))
      raw_path = os.fsencode(path)
      check_fixture_path(raw_path[:-1] + b"\x80")
      for invalid in (path + "s", raw_path + b"\x80", path + "\0"):
        with self.subTest(path=invalid), self.assertRaises(proof.EvidenceError):
          check_fixture_path(invalid)

  def test_late_replacement_before_hold_is_rechecked(self):
    with owned_fixture("direct") as fixture:
      snapshots = []
      duplicate = []
      original = {fixture.child_fd: f"socket:[{PEER}]"}

      def snapshot(child, deadline):
        self.assertIs(child, fixture.child)
        snapshots.append(len(snapshots) + 1)
        if len(snapshots) == 2:
          duplicate.append(replace_fixture_fd(fixture, deadline))
          self.assertEqual(exchange_fixture_peer(fixture, deadline), (proof._PONG, b"CHECKED"))
        if len(snapshots) == 3:
          # macOS has no /proc: model the actual child-reported FD swap.
          return {fixture.child_fd: f"socket:[{PEER + 1}]", duplicate[0]: f"socket:[{PEER}]"}
        return original

      def query(inode, cookie, sequence, deadline):
        return proof.Diagnostic(inode, PEER, COOKIE)

      with mock.patch.object(proof, "_peer_credentials", return_value=fixture.creator), \
           mock.patch.object(proof, "_query_peer", side_effect=query) as queried, \
           mock.patch.object(proof, "_read_child_fds", side_effect=snapshot):
        with self.assertRaisesRegex(proof.EvidenceError, "missing, duplicate, or mismatched peer FD"):
          with proof.calibrate(fixture.client, fixture.child, fixture.child_fd,
                               fixture.creator, fixture.control, fixture.deadline):
            self.fail("mismatched peer FD was accepted")
      self.assertEqual(snapshots, [1, 2, 3])
      self.assertEqual(queried.call_count, 3)
      self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline), (proof._PONG, b"CHECKED"))
      fixture.finish()

  def test_hold_refuses_replacement_after_final_snapshot_and_through_return(self):
    with owned_fixture("direct") as fixture:
      snapshots = []
      links = {fixture.child_fd: f"socket:[{PEER}]"}

      def snapshot(child, deadline):
        self.assertIs(child, fixture.child)
        snapshots.append(len(snapshots) + 1)
        if len(snapshots) == 3:
          proof.send_line(fixture.control, b"REPLACE", deadline)
          self.assertEqual(proof.receive_line(fixture.control, deadline), b"BUSY")
          self.assertEqual(exchange_fixture_peer(fixture, deadline), (proof._PONG, b"CHECKED"))
        return links

      def query(inode, cookie, sequence, deadline):
        return proof.Diagnostic(inode, PEER, COOKIE)

      with mock.patch.object(proof, "_peer_credentials", return_value=fixture.creator), \
           mock.patch.object(proof, "_query_peer", side_effect=query), \
           mock.patch.object(proof, "_read_child_fds", side_effect=snapshot):
        with proof.calibrate(fixture.client, fixture.child, fixture.child_fd,
                             fixture.creator, fixture.control, fixture.deadline) as evidence:
          pinned = evidence.pinned_client
          self.assertEqual(evidence.peer_inode, PEER)
          self.assertEqual(snapshots, [1, 2, 3])
          proof.send_line(fixture.control, b"REPLACE", fixture.deadline)
          self.assertEqual(proof.receive_line(fixture.control, fixture.deadline), b"BUSY")
          self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline, pinned), (proof._PONG, b"CHECKED"))
      self.assertEqual(pinned.fileno(), -1)
      fixture.finish()

  def test_late_client_fd_swap_attests_only_the_pinned_socket(self):
    with owned_fixture("direct") as fixture:
      other, other_peer = socket.socketpair()
      with other, other_peer:
        original_fd = fixture.client.fileno()
        original_inode = proof._socket_identity(fixture.client)[2]
        replacement_inode = os.fstat(other.fileno()).st_ino
        self.assertNotEqual(original_inode, replacement_inode)
        snapshots = []
        credentials = []
        links = {fixture.child_fd: f"socket:[{PEER}]"}

        def snapshot(child, deadline):
          self.assertIs(child, fixture.child)
          snapshots.append(links)
          return links

        def query(inode, cookie, sequence, deadline):
          return proof.Diagnostic(inode, PEER, COOKIE)

        def peer_credentials(client):
          credentials.append(client)
          if len(credentials) == 3:
            self.assertEqual(len(snapshots), 3)
            os.dup2(other.fileno(), original_fd)
          return fixture.creator

        with mock.patch.object(proof, "_peer_credentials", side_effect=peer_credentials), \
             mock.patch.object(proof, "_query_peer", side_effect=query), \
             mock.patch.object(proof, "_read_child_fds", side_effect=snapshot):
          with proof.calibrate(fixture.client, fixture.child, fixture.child_fd,
                               fixture.creator, fixture.control, fixture.deadline) as evidence:
            pinned = evidence.pinned_client
            self.assertEqual(len(credentials), 3)
            self.assertTrue(all(client is pinned for client in credentials))
            self.assertNotEqual(pinned.fileno(), original_fd)
            self.assertEqual(evidence.inode, original_inode)
            self.assertEqual(proof._socket_identity(pinned)[2], original_inode)
            self.assertEqual(os.fstat(original_fd).st_ino, replacement_inode)
            self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline, pinned),
                             (proof._PONG, b"CHECKED"))
            proof.send_line(fixture.control, b"REPLACE", fixture.deadline)
            self.assertEqual(proof.receive_line(fixture.control, fixture.deadline), b"BUSY")
        self.assertEqual(pinned.fileno(), -1)
        fixture.finish()

  def test_child_is_reaped_on_fixture_failure(self):
    child = None
    path = None
    with self.assertRaisesRegex(RuntimeError, "fixture test failure"):
      with owned_fixture("activated") as fixture:
        child = fixture.child
        path = Path(fixture.path)
        raise RuntimeError("fixture test failure")
    self.assertIsNotNone(child.poll())
    self.assertFalse(path.exists())


@unittest.skipUnless(sys.platform == "linux", "Linux-only fake socket integration; hosted Linux CI required")
class LinuxFixtureTests(unittest.TestCase):
  def check_fixture(self, mode):
    with owned_fixture(mode) as fixture:
      with proof.calibrate(fixture.client, fixture.child, fixture.child_fd,
                           fixture.creator, fixture.control, fixture.deadline) as evidence:
        pinned = evidence.pinned_client
        self.assertEqual(proof._socket_identity(pinned)[2], evidence.inode)
        self.assertEqual(evidence.creator, fixture.creator)
        self.assertEqual(evidence.child_fd, fixture.child_fd)
        self.assertNotEqual(evidence.inode, evidence.peer_inode)
        fd_path = f"/proc/{fixture.child.pid}/fd/{fixture.child_fd}"
        self.assertEqual(os.readlink(fd_path), f"socket:[{evidence.peer_inode}]")
        proof.send_line(fixture.control, b"REPLACE", fixture.deadline)
        self.assertEqual(proof.receive_line(fixture.control, fixture.deadline), b"BUSY")
        self.assertEqual(os.readlink(fd_path), f"socket:[{evidence.peer_inode}]")
        self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline, pinned), (proof._PONG, b"CHECKED"))
        if mode == "activated":
          self.assertEqual(evidence.creator[0], os.getpid())
          self.assertNotEqual(evidence.creator[0], fixture.child.pid)
        else:
          self.assertEqual(evidence.creator[0], fixture.child.pid)
      self.assertEqual(pinned.fileno(), -1)
      fixture.finish()

  def test_activator_created_listener_is_accepted_by_child(self):
    self.check_fixture("activated")

  def test_direct_listener_is_created_and_accepted_by_child(self):
    self.check_fixture("direct")

  def test_child_replaces_accepted_fd_during_observation(self):
    for replace_after in (1, 2):
      with self.subTest(replace_after=replace_after), owned_fixture("direct") as fixture:
        read_fds = proof._read_child_fds
        snapshots = []
        replaced = []

        def replace_after_snapshot(child, deadline):
          links = read_fds(child, deadline)
          snapshots.append(links)
          if len(snapshots) == replace_after:
            original_link = links[fixture.child_fd]
            duplicate_fd = replace_fixture_fd(fixture, deadline)
            self.assertEqual(exchange_fixture_peer(fixture, deadline), (proof._PONG, b"CHECKED"))
            changed = read_fds(child, deadline)
            self.assertEqual(changed[duplicate_fd], original_link)
            self.assertNotEqual(changed[fixture.child_fd], original_link)
            replaced.append((original_link, duplicate_fd))
          return links

        with mock.patch.object(proof, "_read_child_fds", side_effect=replace_after_snapshot):
          with self.assertRaisesRegex(proof.EvidenceError, "missing, duplicate, or mismatched peer FD"):
            with proof.calibrate(fixture.client, fixture.child, fixture.child_fd,
                                 fixture.creator, fixture.control, fixture.deadline):
              self.fail("mismatched peer FD was accepted")
        self.assertEqual(len(snapshots), replace_after + 1)
        self.assertEqual(len(replaced), 1)
        original_link, duplicate_fd = replaced[0]
        self.assertNotEqual(os.readlink(f"/proc/{fixture.child.pid}/fd/{fixture.child_fd}"), original_link)
        self.assertEqual(os.readlink(f"/proc/{fixture.child.pid}/fd/{duplicate_fd}"), original_link)
        self.assertEqual(exchange_fixture_peer(fixture, fixture.deadline), (proof._PONG, b"CHECKED"))
        fixture.finish()


if __name__ == "__main__":
  unittest.main()
