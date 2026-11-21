import http.client
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
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

    def serve(self):
        server = gallery.ReviewServer(self.roots, self.fds[2])
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        return server

    def request(self, server, path, method="GET", headers=None):
        conn = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=5)
        conn.request(method, path, headers=headers or {})
        response = conn.getresponse()
        result = response.status, dict(response.getheaders()), response.read()
        conn.close()
        return result

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
