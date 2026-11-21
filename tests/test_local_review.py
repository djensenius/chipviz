import http.client
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock
from urllib.parse import urlsplit


SPEC = importlib.util.spec_from_file_location(
    "local_review", Path(__file__).resolve().parents[1] / "scripts" / "local_review.py"
)
gallery = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(gallery)


class GalleryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "library"
        self.draft = self.base / "draft"
        self.root.mkdir()
        self.draft.mkdir()
        self.fds = [os.open(path, os.O_RDONLY | os.O_DIRECTORY) for path in (self.root, self.draft, gallery.SITE)]
        for fd in self.fds:
            self.addCleanup(os.close, fd)
        self.roots = {"library": self.fds[0], "draft": self.fds[1]}
        self.relative = "modern/pi5/ColourField/quiet.mp4"
        self.write(self.root, self.relative, b"0123456789")

    def write(self, root, relative, data):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def draft_clip(self, look, scenario, profile):
        directory = f"stream-{look}-{scenario}-{profile}"
        self.write(self.draft, f"{directory}/evaluation.mp4", b"draft bytes")
        metadata = {"source_commit": "a" * 40, "command": "private command /machine/path",
                    "profile": "ignored arbitrary metadata"}
        self.write(self.draft, f"{directory}/provenance.json", json.dumps(metadata).encode())
        return directory

    def serve(self, **options):
        server = gallery.ReviewServer(self.roots, self.fds[2], **options)
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server

    def request(self, server, path, method="GET", headers=None, timeout=5):
        conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=timeout)
        try:
            conn.request(method, path, headers=headers or {})
            response = conn.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            conn.close()

    def catalog(self, server):
        status, _, data = self.request(server, f"/{server.token}/catalog.json")
        self.assertEqual(status, 200)
        return json.loads(data)

    def test_drafts_recognized_alongside_general_library(self):
        for look in ("hallway", "house"):
            for scenario in ("quiet", "energetic"):
                for profile in ("m1", "pi5"):
                    self.draft_clip(look, scenario, profile)
        self.write(self.draft, "other/example.mp4", b"another draft")
        catalog, media = gallery.local_catalog(self.roots)
        self.assertEqual(len(catalog["clips"]), 10)
        self.assertEqual(len(media), 10)
        clips = [clip for clip in catalog["clips"] if clip["source_revision"]]
        self.assertEqual(len(clips), 8)
        self.assertEqual({clip["family"] for clip in clips}, {"Hallway", "House"})
        self.assertEqual({clip["scenario"] for clip in clips}, {"quiet", "energetic"})
        for clip in clips:
            self.assertEqual(clip["source_revision"], "a" * 40)
            self.assertIn("not human-reviewed", clip["notes"][1])
            self.assertIn("Mac GPU", clip["capture"])
            if clip["platform"] == "Pi 5 profile":
                self.assertIn("not physical Pi", clip["capture"])
        rendered = json.dumps(catalog)
        for secret in ("private command", "/machine/path", str(self.base), "ignored arbitrary metadata"):
            self.assertNotIn(secret, rendered)
        self.assertEqual(next(c for c in catalog["clips"] if c["collection"] == "library")["family"], "Colour Field")

    def test_metadata_fails_closed_and_old_media_mapping_is_cleared(self):
        directory = self.draft_clip("house", "quiet", "m1")
        metadata = self.draft / directory / "provenance.json"
        server = self.serve()
        prefix = f"/{server.token}/"
        clip = next(c for c in self.catalog(server)["clips"] if c["collection"] == "draft")
        for data in (b"{", b'{"source_commit":"short"}',
                     b'{"source_commit":"' + b"a" * 40 + b'","source_commit":"' + b"b" * 40 + b'"}',
                     b'{"source_commit":null}', b'{"source_commit":NaN}'):
            metadata.write_bytes(data)
            self.assertEqual(self.request(server, prefix + "catalog.json")[0], 404)
            self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)
        metadata.unlink()
        self.assertEqual(self.request(server, prefix + "catalog.json")[0], 404)
        metadata.symlink_to(self.root / self.relative)
        self.assertEqual(self.request(server, prefix + "catalog.json")[0], 404)

    def test_revision_and_unrecognized_drafts_are_conservative(self):
        readme = self.root / "modern" / "README.md"
        readme.write_text("Source revision: `" + "b" * 40 + "`\n", encoding="utf-8")
        self.write(self.draft, "arbitrary/file.mp4", b"sample")
        catalog, _ = gallery.local_catalog(self.roots)
        library, draft = catalog["clips"]
        self.assertEqual(library["source_revision"], "b" * 40)
        self.assertIsNone(draft["source_revision"])
        self.assertIn("not human-reviewed", draft["notes"][1])
        readme.write_text("Revision: " + "b" * 40 + "\nRevision: " + "c" * 40, encoding="utf-8")
        self.assertIsNone(gallery.local_catalog(self.roots)[0]["clips"][0]["source_revision"])

    def test_revision_skips_unrelated_readmes_but_not_ambiguity(self):
        parent = self.root / "modern" / "README.md"
        nested = self.root / "modern" / "pi5" / "README.md"
        sibling = nested.with_suffix(".txt")
        parent.write_text("Revision: " + "b" * 40, encoding="utf-8")
        nested.write_text("Unrelated capture notes.", encoding="utf-8")
        self.assertEqual(gallery.source_revision(self.fds[0], self.relative), "b" * 40)
        sibling.write_text("Source revision: " + "c" * 40, encoding="utf-8")
        self.assertEqual(gallery.source_revision(self.fds[0], self.relative), "c" * 40)
        sibling.unlink()
        nested.write_text("Revision: " + "d" * 40 + "\nSource revision: " + "e" * 40, encoding="utf-8")
        self.assertIsNone(gallery.source_revision(self.fds[0], self.relative))

    def test_revision_invalid_declarations_block_ancestor(self):
        parent = self.root / "modern" / "README.md"
        nested = self.root / "modern" / "pi5" / "README.md"
        parent.write_text("Revision: " + "b" * 40, encoding="utf-8")
        for text in ("Revision: deadbee", "Revision: ", "Source revision: nope",
                     "Revision: " + "c" * 40 + "\nRevision: deadbee",
                     "Revision: " + "c" * 40 + "\nRevision: " + "d" * 40):
            with self.subTest(text=text):
                nested.write_text(text, encoding="utf-8")
                self.assertIsNone(gallery.source_revision(self.fds[0], self.relative))
        nested.write_text("Source integration commit: `" + "C" * 40 + "`.\n", encoding="utf-8")
        self.assertEqual(gallery.source_revision(self.fds[0], self.relative), "c" * 40)

    def test_library_capture_sources_are_unknown(self):
        self.write(self.root, "modern/m1/Lines/quiet.mp4", b"mac")
        self.write(self.root, "retro/nes/native-demo/demo.mp4", b"retro")
        clips = gallery.local_catalog(self.roots)[0]["clips"]
        self.assertEqual({clip["platform"] for clip in clips}, {"Pi 5 profile", "Mac / M1 profile", "NES"})
        for clip in clips:
            self.assertEqual(clip["capture"], "Unknown capture")

    def test_scan_skips_irrelevant_unsafe_names_but_rejects_relevant_ones(self):
        self.write(self.root, "notes 100%.txt", b"notes")
        (self.root / "shortcut 100%").symlink_to(self.base, target_is_directory=True)
        self.assertEqual(len(gallery.local_catalog(self.roots)[0]["clips"]), 1)
        unsafe_dir = self.root / "unsafe%dir"
        unsafe_dir.mkdir()
        with self.assertRaisesRegex(ValueError, "unsafe path"):
            gallery.local_catalog(self.roots)
        unsafe_dir.rmdir()
        self.write(self.root, "unsafe%.mp4", b"video")
        with self.assertRaisesRegex(ValueError, "unsafe path"):
            gallery.local_catalog(self.roots)

    def test_scan_entry_depth_and_byte_limits(self):
        for limits, message in (
            (gallery.ScanLimits(max_entries=1), "entry limit"),
            (gallery.ScanLimits(max_depth=1), "depth limit"),
            (gallery.ScanLimits(max_bytes=9), "byte limit"),
        ):
            with self.subTest(message=message):
                budget = gallery.ScanBudget(limits)
                with self.assertRaisesRegex(ValueError, message):
                    gallery.local_catalog(self.roots, budget)
                if message == "entry limit":
                    self.assertEqual(budget.entries, 2)

    def test_hash_byte_budget_is_shared_across_roots(self):
        self.write(self.draft, "other/example.mp4", b"four")
        budget = gallery.ScanBudget(gallery.ScanLimits(max_bytes=13))
        with self.assertRaisesRegex(ValueError, "byte limit"):
            gallery.local_catalog(self.roots, budget)
        self.assertEqual(budget.bytes, 10)
        budget = gallery.ScanBudget(gallery.ScanLimits(max_bytes=14))
        self.assertEqual(len(gallery.local_catalog(self.roots, budget)[0]["clips"]), 2)
        self.assertEqual(budget.bytes, 14)

    def test_hash_byte_budget_checks_growth_before_hashing(self):
        video = self.write(self.root, self.relative, b"x")
        stream = gallery.open_file(self.fds[0], self.relative)

        class GrowingStream:
            def __enter__(self):
                return self

            def __exit__(self, *_):
                stream.close()

            def fileno(self):
                return stream.fileno()

            def read(self, size):
                with video.open("ab") as writer:
                    writer.write(b"more")
                return stream.read(size)

        budget = gallery.ScanBudget(gallery.ScanLimits(max_bytes=2))
        with mock.patch.object(gallery, "open_file", return_value=GrowingStream()):
            with self.assertRaisesRegex(ValueError, "byte limit"):
                gallery.digest_file(self.fds[0], self.relative, "library", budget)
        self.assertEqual(budget.bytes, 0)

    def test_scan_deadline_checked_during_enumeration(self):
        now = [0]
        budget = gallery.ScanBudget(gallery.ScanLimits(max_seconds=1), clock=lambda: now[0])
        original_stat = os.stat

        def delayed_stat(*args, **kwargs):
            result = original_stat(*args, **kwargs)
            now[0] = 2
            return result

        with mock.patch.object(gallery.os, "stat", side_effect=delayed_stat):
            with self.assertRaisesRegex(ValueError, "time limit"):
                gallery.local_catalog(self.roots, budget)

    def test_scan_cancellation_checked_during_hashing(self):
        self.write(self.root, self.relative, b"x" * (2 * 1024 * 1024))
        cancelled = threading.Event()

        class CancelAfterBlock(gallery.ScanBudget):
            def hash_bytes(self, count):
                super().hash_bytes(count)
                if self.bytes >= 1024 * 1024:
                    cancelled.set()

        budget = CancelAfterBlock(cancel=cancelled)
        with self.assertRaisesRegex(ValueError, "scan cancelled"):
            gallery.digest_file(self.fds[0], self.relative, "library", budget)
        self.assertEqual(budget.bytes, 1024 * 1024)

    def test_scan_deadline_checked_during_hashing(self):
        self.write(self.root, self.relative, b"x" * (2 * 1024 * 1024))
        now = [0]

        class ExpireAfterBlock(gallery.ScanBudget):
            def hash_bytes(self, count):
                super().hash_bytes(count)
                if self.bytes >= 1024 * 1024:
                    now[0] = 2

        budget = ExpireAfterBlock(gallery.ScanLimits(max_seconds=1), clock=lambda: now[0])
        with self.assertRaisesRegex(ValueError, "time limit"):
            gallery.digest_file(self.fds[0], self.relative, "library", budget)
        self.assertEqual(budget.bytes, 1024 * 1024)

    def test_limit_failure_clears_previous_index(self):
        server = self.serve()
        prefix = f"/{server.token}/"
        clip = self.catalog(server)["clips"][0]
        server.scan_limits = gallery.ScanLimits(max_bytes=9)
        self.assertEqual(self.request(server, prefix + "catalog.json")[0], 404)
        self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)
        server.scan_limits = gallery.ScanLimits()
        self.assertEqual(self.catalog(server)["clips"][0]["id"], clip["id"])

    def test_incomplete_request_does_not_block_capability_request(self):
        started = threading.Event()
        original = gallery.Handler.handle_one_request

        def mark_started(handler):
            started.set()
            return original(handler)

        with mock.patch.object(gallery.Handler, "handle_one_request", mark_started):
            server = self.serve(header_timeout=5)
            with socket.create_connection(("127.0.0.1", server.server_port), timeout=2) as partial:
                partial.sendall(b"GET / HTTP/1.1\r\nHost: 127.0.0.1")
                self.assertTrue(started.wait(2))
                status, _, _ = self.request(server, f"/{server.token}/", timeout=1)
                self.assertEqual(status, 200)

    def test_header_deadline_bounds_slow_drip_with_controlled_clock(self):
        connection = mock.Mock(spec=["settimeout", "recv_into"])

        def drip(buffer):
            buffer[:1] = b"x"
            return 1

        connection.recv_into.side_effect = drip
        with mock.patch.object(gallery.time, "monotonic", side_effect=[100.0, 100.0, 100.3, 100.7, 100.81]):
            reader = gallery.HeaderReader(connection, timeout=0.8, io_timeout=2)
            for _ in range(3):
                buffer = bytearray(4)
                self.assertEqual(reader.readinto(buffer), 1)
                self.assertEqual(buffer[:1], b"x")
            with self.assertRaisesRegex(TimeoutError, "request header deadline"):
                reader.readinto(bytearray(4))
        self.assertEqual(connection.recv_into.call_count, 3)
        self.assertEqual(reader.remaining, 65533)
        for actual, expected in zip(connection.settimeout.call_args_list, (0.8, 0.5, 0.1)):
            self.assertAlmostEqual(actual.args[0], expected)
        self.assertEqual(connection.settimeout.call_count, 3)

    def test_delayed_client_tolerates_peer_close_after_header_deadline(self):
        closed = threading.Event()
        original = gallery.ReviewServer.shutdown_request

        def mark_closed(server, request):
            try:
                original(server, request)
            finally:
                closed.set()

        server = self.serve(header_timeout=0.8, io_timeout=2)
        with mock.patch.object(gallery.ReviewServer, "shutdown_request", mark_closed):
            with socket.create_connection(("127.0.0.1", server.server_port), timeout=5) as partial:
                partial.settimeout(5)
                try:
                    partial.sendall(b"G")
                except ConnectionError:
                    pass  # The scheduled client may already have missed the deadline.
                self.assertTrue(closed.wait(10))
                try:
                    partial.sendall(b"E")  # Simulate the client resuming after the peer closed.
                except ConnectionError:
                    pass
                try:
                    data = partial.recv(1)
                except ConnectionError:
                    data = b""
                self.assertEqual(data, b"")
        self.assertEqual(self.request(server, f"/{server.token}/")[0], 200)

    def test_oversized_request_headers_are_closed(self):
        server = self.serve()
        with socket.create_connection(("127.0.0.1", server.server_port), timeout=2) as partial:
            partial.settimeout(2)
            partial.sendall(b"GET / HTTP/1.1\r\nX-Long: " + b"x" * 65536)
            try:
                data = partial.recv(1)
            except ConnectionResetError:
                data = b""
            self.assertEqual(data, b"")
        self.assertEqual(self.request(server, f"/{server.token}/")[0], 200)

    def test_worker_limit_closes_excess_connections_and_recovers(self):
        started = threading.Event()
        count_lock = threading.Lock()
        count = 0
        original = gallery.Handler.handle_one_request

        def mark_started(handler):
            nonlocal count
            with count_lock:
                count += 1
                if count == 2:
                    started.set()
            return original(handler)

        with mock.patch.object(gallery.Handler, "handle_one_request", mark_started):
            server = self.serve(max_workers=2, header_timeout=5)
            held = [socket.create_connection(("127.0.0.1", server.server_port), timeout=2) for _ in range(2)]
            try:
                for connection in held:
                    connection.sendall(b"GET /")
                self.assertTrue(started.wait(2))
                with socket.create_connection(("127.0.0.1", server.server_port), timeout=2) as excess:
                    excess.settimeout(2)
                    self.assertEqual(excess.recv(1), b"")
            finally:
                for connection in held:
                    connection.close()
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline:
                with server.connections_lock:
                    if not server.connections:
                        break
                time.sleep(0.01)
            self.assertEqual(self.request(server, f"/{server.token}/")[0], 200)

    def test_worker_start_failure_releases_capacity(self):
        server = self.serve(max_workers=1)
        with mock.patch.object(gallery.threading.Thread, "start", side_effect=RuntimeError("unavailable")):
            with self.assertRaises((OSError, http.client.HTTPException)):
                self.request(server, f"/{server.token}/")
        self.assertEqual(self.request(server, f"/{server.token}/")[0], 200)

    def test_overlapping_refreshes_do_not_starve_playback(self):
        server = self.serve(header_timeout=30)
        prefix = f"/{server.token}/"
        self.write(self.root, "other/remove.mp4", b"remove")
        clips = self.catalog(server)["clips"]
        clip = next(c for c in clips if c["family"] == "Colour Field")
        removed = next(c for c in clips if c["title"] == "remove")
        entered = threading.Event()
        release = threading.Event()
        original_catalog = gallery.local_catalog
        original_handle = gallery.Handler.handle_one_request
        ready = threading.Condition()
        ready_count = 0
        scans = 0
        results = []

        def mark_ready(handler):
            nonlocal ready_count
            with ready:
                ready_count += 1
                ready.notify_all()
            return original_handle(handler)

        def blocked_catalog(roots, budget):
            nonlocal scans
            scans += 1
            entered.set()
            if not release.wait(30):
                raise ValueError("test timed out")
            return original_catalog(roots, budget)

        def fetch_catalog():
            results.append(self.request(server, prefix + "catalog.json", timeout=30)[0])

        with mock.patch.object(gallery, "local_catalog", blocked_catalog), \
                mock.patch.object(gallery.Handler, "handle_one_request", mark_ready):
            first = threading.Thread(target=fetch_catalog)
            first.start()
            connections = []
            try:
                self.assertTrue(entered.wait(2))
                for number in range(15):
                    conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=10)
                    connections.append(conn)
                    conn.connect()
                    with ready:
                        self.assertTrue(ready.wait_for(lambda: ready_count >= number + 2, timeout=10))
                gate = threading.Barrier(16)
                outcomes, errors = [], []

                def overlap(conn):
                    try:
                        gate.wait(10)
                        conn.request("GET", prefix + "catalog.json")
                        response = conn.getresponse()
                        outcomes.append((response.status, response.getheader("Retry-After")))
                        response.read()
                    except Exception as exc:
                        errors.append(str(exc))
                    finally:
                        conn.close()

                workers = [threading.Thread(target=overlap, args=(conn,)) for conn in connections]
                for worker in workers:
                    worker.start()
                gate.wait(10)
                for worker in workers:
                    worker.join(10)
                self.assertFalse(any(worker.is_alive() for worker in workers))
                self.assertEqual(errors, [])
                self.assertEqual(outcomes, [(503, "1")] * 15)
                with server.connections_lock:
                    self.assertTrue(server.connections_lock.wait_for(lambda: len(server.connections) == 1, timeout=10))
                gate = threading.Barrier(9)
                outcomes, errors = [], []

                def burst():
                    try:
                        gate.wait(10)
                        status, headers, _ = self.request(server, prefix + "catalog.json")
                        outcomes.append((status, headers.get("Retry-After")))
                    except Exception as exc:
                        errors.append(str(exc))

                workers = [threading.Thread(target=burst) for _ in range(8)]
                for worker in workers:
                    worker.start()
                gate.wait(10)
                for worker in workers:
                    worker.join(10)
                self.assertFalse(any(worker.is_alive() for worker in workers))
                self.assertEqual(errors, [])
                self.assertEqual(outcomes, [(503, "1")] * 8)
                status, headers, data = self.request(
                    server, prefix + "media/" + clip["id"], headers={"Range": "bytes=2-5"}
                )
                self.assertEqual((status, headers["Content-Range"], data), (206, "bytes 2-5/10", b"2345"))
                self.write(self.root, self.relative, b"changed")
                (self.root / "other" / "remove.mp4").unlink()
                self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 409)
                self.assertEqual(self.request(server, prefix + "media/" + removed["id"])[0], 404)
                self.assertEqual(self.request(server, prefix)[0], 200)
            finally:
                for conn in connections:
                    conn.close()
                release.set()
                first.join(5)
            self.assertFalse(first.is_alive())
            self.assertEqual(scans, 1)
            self.assertEqual(results, [200])
        self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)
        replacement = self.catalog(server)["clips"][0]
        self.assertNotEqual(replacement["id"], clip["id"])
        self.assertEqual(self.request(server, prefix + "media/" + replacement["id"])[2], b"changed")

    def test_failed_refresh_invalidates_prior_media_until_next_success(self):
        server = self.serve()
        prefix = f"/{server.token}/"
        clip = self.catalog(server)["clips"][0]
        first_entered, second_entered = threading.Event(), threading.Event()
        release_first, release_second = threading.Event(), threading.Event()
        original_catalog = gallery.local_catalog
        scans = 0
        statuses = []

        def blocked_catalog(roots, budget):
            nonlocal scans
            scans += 1
            if scans == 1:
                first_entered.set()
                if not release_first.wait(5):
                    raise ValueError("test timed out")
                raise ValueError("invalid catalog")
            second_entered.set()
            if not release_second.wait(5):
                raise ValueError("test timed out")
            return original_catalog(roots, budget)

        def fetch_catalog():
            statuses.append(self.request(server, prefix + "catalog.json")[0])

        with mock.patch.object(gallery, "local_catalog", blocked_catalog):
            first = threading.Thread(target=fetch_catalog)
            second = threading.Thread(target=fetch_catalog)
            first.start()
            try:
                self.assertTrue(first_entered.wait(2))
                status, headers, _ = self.request(server, prefix + "catalog.json")
                self.assertEqual((status, headers["Retry-After"]), (503, "1"))
                status, _, data = self.request(
                    server, prefix + "media/" + clip["id"], headers={"Range": "bytes=2-5"}
                )
                self.assertEqual((status, data), (206, b"2345"))
                release_first.set()
                first.join(5)
                self.assertEqual(statuses, [404])
                self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)
                second.start()
                self.assertTrue(second_entered.wait(2))
                self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)
            finally:
                release_first.set()
                release_second.set()
                first.join(5)
                if second.ident is not None:
                    second.join(5)
            self.assertFalse(first.is_alive())
            self.assertFalse(second.is_alive())
            self.assertEqual(statuses, [404, 200])
        self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[2], b"0123456789")

    def test_unexpected_catalog_failure_invalidates_prior_media(self):
        server = self.serve()
        prefix = f"/{server.token}/"
        clip = self.catalog(server)["clips"][0]
        with mock.patch.object(gallery, "local_catalog", side_effect=RuntimeError("scan failed")):
            with self.assertRaises((OSError, http.client.HTTPException)):
                self.request(server, prefix + "catalog.json")
        self.assertEqual(self.request(server, prefix + "media/" + clip["id"])[0], 404)

    def test_shutdown_waits_for_catalog_worker_before_closing_roots(self):
        server = self.serve()
        entered = threading.Event()
        release = threading.Event()
        closed = threading.Event()
        cancelled = threading.Event()
        expected_identity = gallery.directory_identity(self.fds[0])
        original = gallery.local_catalog

        def blocked_catalog(roots, budget):
            entered.set()
            if not release.wait(5):
                raise ValueError("test timed out")
            try:
                return original(roots, budget)
            except ValueError as exc:
                if str(exc) == "scan cancelled":
                    cancelled.set()
                raise

        def fetch():
            try:
                self.request(server, f"/{server.token}/catalog.json")
            except (OSError, http.client.HTTPException):
                pass

        def close_server():
            server.server_close()
            closed.set()

        with mock.patch.object(gallery, "local_catalog", blocked_catalog):
            client = threading.Thread(target=fetch)
            client.start()
            closer = None
            try:
                self.assertTrue(entered.wait(2))
                server.shutdown()
                closer = threading.Thread(target=close_server)
                closer.start()
                self.assertFalse(closed.wait(0.1))
            finally:
                release.set()
                client.join(5)
                if closer is not None:
                    closer.join(5)
            self.assertTrue(closed.is_set())
            self.assertTrue(cancelled.is_set())
            self.assertFalse(client.is_alive())
            self.assertEqual(gallery.directory_identity(self.fds[0]), expected_identity)

    def test_shutdown_interrupts_incomplete_connections(self):
        started = threading.Event()
        original = gallery.Handler.handle_one_request

        def mark_started(handler):
            started.set()
            return original(handler)

        with mock.patch.object(gallery.Handler, "handle_one_request", mark_started):
            server = self.serve(header_timeout=10)
            with socket.create_connection(("127.0.0.1", server.server_port), timeout=2) as partial:
                partial.sendall(b"GET /")
                self.assertTrue(started.wait(2))
                server.shutdown()
                closer = threading.Thread(target=server.server_close)
                closer.start()
                closer.join(2)
                self.assertFalse(closer.is_alive())

    def test_capability_host_paths_symlinks_and_headers(self):
        outside = self.base / "outside.mp4"
        outside.write_bytes(b"secret")
        (self.root / "escape.mp4").symlink_to(outside)
        (self.root / "link").symlink_to(self.base, target_is_directory=True)
        server = self.serve()
        self.assertEqual(server.server_address[0], "127.0.0.1")
        prefix = f"/{server.token}/"
        for path in ("/", "/wrong/catalog.json", prefix + "../outside.mp4",
                     prefix + "%2e%2e/outside.mp4", prefix + "%252e%252e/outside.mp4",
                     prefix + "media/escape.mp4", prefix + "media/link/outside.mp4",
                     prefix + "media/", prefix + "?x=1", prefix + "index.html/"):
            status, headers, body = self.request(server, path)
            self.assertEqual(status, 404, path)
            self.assertNotIn(server.token.encode(), body)
            self.assertEqual(headers["Referrer-Policy"], "no-referrer")
            self.assertNotIn("Access-Control-Allow-Origin", headers)
        self.assertEqual(self.request(server, prefix, headers={"Host": "evil.example"})[0], 404)
        status, headers, _ = self.request(server, prefix)
        self.assertEqual(status, 200)
        self.assertEqual(headers["Cache-Control"], "no-store")
        self.assertIn("frame-ancestors 'none'", headers["Content-Security-Policy"])
        self.assertEqual(len(self.catalog(server)["clips"]), 1)
        for name in ("escape.mp4", "link/outside.mp4", "../outside.mp4"):
            with self.assertRaises((OSError, ValueError)):
                gallery.open_file(self.fds[0], name)

    def test_ranges_head_refresh_and_changed_file(self):
        server = self.serve()
        prefix = f"/{server.token}/"
        clip = self.catalog(server)["clips"][0]
        path = prefix + "media/" + clip["id"]
        status, headers, data = self.request(server, path, headers={"Range": "bytes=2-5"})
        self.assertEqual((status, data, headers["Content-Range"]), (206, b"2345", "bytes 2-5/10"))
        self.assertEqual(self.request(server, path, headers={"Range": "bytes=-3"})[2], b"789")
        status, headers, data = self.request(server, path, "HEAD", {"Range": "bytes=2-"})
        self.assertEqual((status, headers["Content-Length"], data), (206, "8", b""))
        for value in ("bytes=100-", "bytes=-0", "bytes=4-2", "bytes=1-2,4-5", "bytes=-"):
            self.assertEqual(self.request(server, path, headers={"Range": value})[0], 416)
        self.assertEqual(self.request(server, path, headers={"Range": "bytes=-" + "9" * 5000})[2], b"0123456789")
        self.assertEqual(self.request(server, path, headers={"Range": "bytes=" + "9" * 5000 + "-"})[0], 416)
        self.assertEqual(self.request(server, path)[2], b"0123456789")
        self.write(self.root, self.relative, b"changed")
        self.assertEqual(self.request(server, path)[0], 409)
        self.write(self.root, "retro/snes/new.mp4", b"new")
        clips = self.catalog(server)["clips"]
        self.assertEqual(len(clips), 2)
        self.assertNotEqual(next(c for c in clips if c["family"] == "Colour Field")["id"], clip["id"])
        self.assertEqual(self.request(server, path)[0], 404)

    def test_replacing_catalogued_file_with_symlink_cannot_escape(self):
        outside = self.base / "outside.mp4"
        outside.write_bytes(b"secret")
        server = self.serve()
        clip = self.catalog(server)["clips"][0]
        video = self.root / self.relative
        video.unlink()
        video.symlink_to(outside)
        status, _, data = self.request(server, f"/{server.token}/media/{clip['id']}")
        self.assertEqual(status, 404)
        self.assertNotIn(b"secret", data)

    def test_command_line_starts_loopback_server_with_both_roots(self):
        self.draft_clip("house", "quiet", "pi5")
        process = subprocess.Popen(
            [sys.executable, str(Path(gallery.__file__)), "--root", str(self.root),
             "--draft-root", str(self.draft)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        try:
            url = urlsplit(process.stdout.readline().strip())
            self.assertEqual((url.scheme, url.hostname), ("http", "127.0.0.1"))
            conn = http.client.HTTPConnection(url.hostname, url.port, timeout=5)
            try:
                conn.request("GET", url.path + "catalog.json")
                response = conn.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(len(json.loads(response.read())["clips"]), 2)
            finally:
                conn.close()
        finally:
            process.terminate()
            process.communicate(timeout=5)

    def test_command_line_interrupt_closes_active_connection(self):
        process = subprocess.Popen(
            [sys.executable, str(Path(gallery.__file__)), "--root", str(self.root)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        try:
            url = urlsplit(process.stdout.readline().strip())
            with socket.create_connection((url.hostname, url.port), timeout=2) as partial:
                partial.sendall(b"GET /")
                conn = http.client.HTTPConnection(url.hostname, url.port, timeout=2)
                try:
                    conn.request("GET", url.path)
                    response = conn.getresponse()
                    self.assertEqual(response.status, 200)
                    response.read()
                finally:
                    conn.close()
                process.send_signal(signal.SIGINT)
                stdout, stderr = process.communicate(timeout=3)
                self.assertEqual((process.returncode, stdout, stderr), (0, "", ""))
        finally:
            if process.poll() is None:
                process.terminate()
                process.communicate(timeout=5)

    def test_command_line_rejects_overlapping_roots(self):
        nested_library = self.root / "nested"
        nested_draft = self.draft / "nested"
        nested_library.mkdir()
        nested_draft.mkdir()
        alias = self.base / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        for library, draft in (
            (self.root, self.root), (self.root, nested_library),
            (nested_library, self.root), (self.draft, nested_draft),
            (nested_draft, self.draft), (self.root, alias / "nested"),
            (alias / "nested", self.root),
        ):
            with self.subTest(library=library, draft=draft):
                result = subprocess.run(
                    [sys.executable, str(Path(gallery.__file__)), "--root", str(library),
                     "--draft-root", str(draft)], capture_output=True, text=True, timeout=5
                )
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stdout, "")
                self.assertIn("separate and non-overlapping", result.stderr)

    def test_directory_ancestry_uses_identity_not_string_prefix(self):
        sibling = self.base / "library-other"
        sibling.mkdir()
        fd = os.open(sibling, os.O_RDONLY | os.O_DIRECTORY)
        try:
            self.assertFalse(gallery.directory_contains(self.fds[0], fd))
            self.assertFalse(gallery.directory_contains(fd, self.fds[0]))
        finally:
            os.close(fd)

    def test_directory_ancestry_errors_fail_closed(self):
        with mock.patch.object(gallery.os, "open", side_effect=PermissionError("unavailable")):
            with self.assertRaises(PermissionError):
                gallery.directory_contains(self.fds[0], self.fds[1])

    def test_changed_during_hash_rejected(self):
        original = gallery.fingerprint
        count = 0

        def changed(st):
            nonlocal count
            count += 1
            value = original(st)
            return value if count == 1 else (*value[:-1], value[-1] + 1)

        with mock.patch.object(gallery, "fingerprint", side_effect=changed):
            with self.assertRaisesRegex(ValueError, "changed while hashing"):
                gallery.digest_file(self.fds[0], self.relative, "library")


if __name__ == "__main__":
    unittest.main()
