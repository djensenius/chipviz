#!/usr/bin/env python3
"""Opt-in, read-only loopback video review gallery."""

import argparse
import errno
import hashlib
import io
import json
import os
from pathlib import Path
import re
import secrets
import signal
import socket
import stat
import threading
import time
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import unquote, urlsplit


SITE = Path(__file__).resolve().parent.parent / "scripts" / "local-review"
DEFAULT_ROOT = Path.home() / "Desktop" / "chipviz"
REVISION = re.compile(r"[0-9a-f]{40}\Z")
CLIP_ID = re.compile(r"clip-[0-9a-f]{20}\Z")
DRAFT = re.compile(r"stream-(hallway|house)-(quiet|energetic)-(m1|pi5)/evaluation\.mp4\Z")
FAMILIES = {
    "signalfield": "Signal Field", "spectrumvolume": "Spectrum Volume",
    "pulsetunnel": "Pulse Tunnel", "lines": "Lines", "static": "Static",
    "object": "Object", "colourfield": "Colour Field", "house": "House",
    "starfield": "Starfield",
}
PLATFORMS = {
    "m1": "Mac / M1 profile", "pi5": "Pi 5 profile", "game-boy": "Game Boy",
    "nes": "NES", "c64": "C64", "gba": "GBA", "n64": "N64", "snes": "SNES",
    "genesis": "Genesis", "master-system": "Master System", "sms": "Master System",
}
FLAGS = os.O_RDONLY | os.O_NOFOLLOW


@dataclass(frozen=True)
class ScanLimits:
    max_entries: int = 50000
    max_depth: int = 32
    max_bytes: int = 16 * 1024 ** 3
    max_seconds: float = 300


class ScanBudget:
    def __init__(self, limits=None, cancel=None, clock=None):
        self.limits = limits or ScanLimits()
        self.cancel = cancel
        self.clock = clock or time.monotonic
        self.deadline = self.clock() + self.limits.max_seconds
        self.entries = 0
        self.bytes = 0

    def check(self):
        if self.cancel is not None and self.cancel.is_set():
            raise ValueError("scan cancelled")
        if self.clock() >= self.deadline:
            raise ValueError("scan time limit exceeded")

    def visit(self):
        self.check()
        self.entries += 1
        if self.entries > self.limits.max_entries:
            raise ValueError("scan entry limit exceeded")

    def hash_bytes(self, count):
        self.check()
        if count > self.limits.max_bytes - self.bytes:
            raise ValueError("scan byte limit exceeded")
        self.bytes += count


def parts(path):
    if not path or "\\" in path or "%" in path or any(ord(c) < 32 or ord(c) == 127 for c in path):
        raise ValueError("unsafe path")
    names = path.split("/")
    if any(name in ("", ".", "..") for name in names):
        raise ValueError("unsafe path")
    return names


def open_file(root_fd, relative):
    """Open only regular files beneath an already opened root, without symlinks."""
    names = parts(relative)
    fd = os.dup(root_fd)
    try:
        for name in names[:-1]:
            next_fd = os.open(name, FLAGS | os.O_DIRECTORY, dir_fd=fd)
            os.close(fd)
            fd = next_fd
        file_fd = os.open(names[-1], FLAGS | os.O_NONBLOCK, dir_fd=fd)
        if not stat.S_ISREG(os.fstat(file_fd).st_mode):
            os.close(file_fd)
            raise ValueError("not a regular file")
        return os.fdopen(file_fd, "rb")
    finally:
        os.close(fd)


def fingerprint(st):
    return (st.st_dev, st.st_ino, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def digest_file(root_fd, relative, namespace, budget=None):
    budget = budget or ScanBudget()
    budget.check()
    with open_file(root_fd, relative) as stream:
        info = os.fstat(stream.fileno())
        before = fingerprint(info)
        if info.st_size > budget.limits.max_bytes - budget.bytes:
            raise ValueError("scan byte limit exceeded")
        digest = hashlib.sha256()
        while True:
            budget.check()
            block = stream.read(1024 * 1024)
            budget.hash_bytes(len(block))
            if not block:
                break
            digest.update(block)
        budget.check()
        if before != fingerprint(os.fstat(stream.fileno())):
            raise ValueError("file changed while hashing")
    raw = digest.digest()
    identity = namespace.encode() + b"\0" + relative.encode("utf-8") + b"\0" + raw
    return "clip-" + hashlib.sha256(identity).hexdigest()[:20], raw.hex(), before


def scan(root_fd, budget, prefix="", depth=0):
    """Walk opened directories, never following symlinks, and yield MP4 names."""
    budget.check()
    with os.scandir(root_fd) as entries:
        names = []
        for item in entries:
            budget.visit()
            info = os.stat(item.name, dir_fd=root_fd, follow_symlinks=False)
            if stat.S_ISLNK(info.st_mode):
                continue
            is_dir = stat.S_ISDIR(info.st_mode)
            if not is_dir and not (stat.S_ISREG(info.st_mode) and item.name.lower().endswith(".mp4")):
                continue
            relative = prefix + item.name
            parts(relative)
            names.append((item.name, is_dir))
    for name, is_dir in sorted(names):
        budget.check()
        relative = prefix + name
        if is_dir:
            if depth >= budget.limits.max_depth:
                raise ValueError("scan depth limit exceeded")
            fd = os.open(name, FLAGS | os.O_DIRECTORY, dir_fd=root_fd)
            try:
                yield from scan(fd, budget, relative + "/", depth + 1)
            finally:
                os.close(fd)
        else:
            yield relative


def read_small(root_fd, relative, budget=None):
    if budget:
        budget.check()
    with open_file(root_fd, relative) as stream:
        before = fingerprint(os.fstat(stream.fileno()))
        data = stream.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024 or before != fingerprint(os.fstat(stream.fileno())):
            raise ValueError("invalid or changing metadata")
    if budget:
        budget.check()
    return data.decode("utf-8")


def source_revision(root_fd, relative, budget=None):
    names = parts(relative)
    for length in range(len(names) - 1, -1, -1):
        revisions = set()
        for filename in ("README.md", "README.txt"):
            try:
                text = read_small(root_fd, "/".join(names[:length] + [filename]), budget)
            except OSError as exc:
                if exc.errno == errno.ENOENT:
                    continue
                raise
            declarations = re.findall(
                r"(?im)^[ \t]*(?:[-*][ \t]*)?(?:source revision|source integration commit|revision)"
                r"[ \t]*:[ \t]*(.*)$", text
            )
            for value in declarations:
                match = re.fullmatch(r"(?:`([0-9a-f]{40})`|([0-9a-f]{40}))[ \t]*[.,;:!?]*",
                                     value.strip(), re.I)
                if not match:
                    return None
                revisions.add((match.group(1) or match.group(2)).lower())
        if revisions:
            return revisions.pop() if len(revisions) == 1 else None
    return None


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate metadata key")
        result[key] = value
    return result


def invalid_constant(_value):
    raise ValueError("invalid metadata constant")


def draft_revision(root_fd, relative, budget=None):
    metadata = json.loads(read_small(root_fd, relative.rsplit("/", 1)[0] + "/provenance.json", budget),
                          object_pairs_hook=unique_object, parse_constant=invalid_constant)
    if not isinstance(metadata, dict) or not isinstance(metadata.get("source_commit"), str):
        raise ValueError("missing draft source commit")
    revision = metadata["source_commit"]
    if not REVISION.fullmatch(revision):
        raise ValueError("invalid draft source commit")
    return revision


def entry(root_fd, relative, namespace, budget):
    clip_id, sha, stamp = digest_file(root_fd, relative, namespace, budget)
    names = parts(relative)
    title = Path(names[-1]).stem
    family = platform = variant = "Unknown"
    scenario = title
    capture = "Unknown capture"
    revision = None
    notes = ["Labels are inferred from filenames; provenance and review status are not verified."]
    match = DRAFT.fullmatch(relative) if namespace == "draft" else None
    if match:
        look, scenario, profile = match.groups()
        family = look.title()
        title = f"{family} / {scenario.title()} / {profile.upper()}"
        platform = PLATFORMS[profile]
        variant = "Hallway surface" if look == "hallway" else "House"
        capture = ("Pi 5 profile rendered on Mac GPU; not physical Pi hardware" if profile == "pi5"
                   else "M1 profile rendered on Mac GPU")
        revision = draft_revision(root_fd, relative, budget)
    elif len(names) >= 4 and names[0].lower() == "modern":
        code = names[1].lower()
        platform = PLATFORMS.get(code, names[1])
        family = FAMILIES.get(names[2].lower(), names[2])
        variant = names[3] if len(names) >= 5 else ("Original" if family == "Pulse Tunnel" else "Unknown")
    elif len(names) >= 3 and names[0].lower() == "retro":
        code = names[1].lower()
        platform = PLATFORMS.get(code, names[1])
        family = names[2] if len(names) >= 4 else "Unknown"
        if family.lower().startswith("colour-field"):
            family = "Colour Field"
            if len(names) >= 5:
                variant = names[3]
        if len(names) == 3 and code == "gba":
            family, variant = "Blockscape", title.split("-", 1)[0].lower()
        if family.lower() == "native-demo":
            family = {"game-boy": "Tile Scroll", "nes": "Diamond Scroll"}.get(code, family)
        if family.lower() == "native-petscii" and code == "c64":
            family = "PETSCII Meter Field"
        if code in ("n64", "snes"):
            family = {"n64": "Perspective Meters", "snes": "Tile Meters"}[code]
        if code in ("genesis", "master-system", "sms") and "vdp-seed" in relative.lower():
            family = "Static diagnostic"
    if not match:
        revision = source_revision(root_fd, relative, budget)
    if namespace == "draft":
        notes.append("Review-only draft; not human-reviewed. Do not publish. Normal-speed human review and safety/flash assessment remain required.")
    return ({
        "id": clip_id, "sha256": sha, "title": title, "family": family,
        "platform": platform, "variant": variant, "scenario": scenario,
        "capture": capture, "source_revision": revision, "collection": namespace, "notes": notes,
    }, stamp)


def local_catalog(roots, budget=None):
    budget = budget or ScanBudget()
    clips, media = [], {}
    for namespace, root_fd in roots.items():
        for relative in scan(root_fd, budget):
            clip, stamp = entry(root_fd, relative, namespace, budget)
            if clip["id"] in media:
                raise ValueError("duplicate clip identity")
            media[clip["id"]] = (root_fd, relative, stamp)
            clips.append(clip)
    budget.check()
    return {"schema_version": 1, "clips": clips}, media


def directory_identity(fd):
    info = os.fstat(fd)
    return info.st_dev, info.st_ino


def directory_contains(ancestor_fd, child_fd):
    """Compare opened directory identities while walking towards the filesystem root."""
    ancestor = directory_identity(ancestor_fd)
    fd = os.dup(child_fd)
    seen = set()
    try:
        while True:
            current = directory_identity(fd)
            if current == ancestor:
                return True
            if current in seen:
                raise ValueError("directory ancestry cycle")
            seen.add(current)
            parent_fd = os.open("..", FLAGS | os.O_DIRECTORY, dir_fd=fd)
            try:
                parent = directory_identity(parent_fd)
            except BaseException:
                os.close(parent_fd)
                raise
            if parent == current:
                os.close(parent_fd)
                return False
            os.close(fd)
            fd = parent_fd
    finally:
        os.close(fd)


class HeaderReader(io.RawIOBase):
    """Bound the entire request header, including clients that send slowly."""
    def __init__(self, connection, timeout, io_timeout):
        super().__init__()
        self.connection = connection
        self.deadline = time.monotonic() + timeout
        self.io_timeout = io_timeout
        self.remaining = 65536

    def readable(self):
        return True

    def readinto(self, buffer):
        if self.deadline is None:
            return self.connection.recv_into(buffer)
        remaining_time = self.deadline - time.monotonic()
        if remaining_time <= 0:
            raise TimeoutError("request header deadline")
        if self.remaining == 0:
            raise OSError("request headers too large")
        self.connection.settimeout(min(self.io_timeout, remaining_time))
        count = self.connection.recv_into(memoryview(buffer)[:self.remaining])
        self.remaining -= count
        return count

    def finish_headers(self):
        self.deadline = None
        self.connection.settimeout(self.io_timeout)


class ReviewServer(HTTPServer):
    # Keep short refresh bursts from overrunning the default listen backlog.
    request_queue_size = 32

    def __init__(self, roots, site_fd, port=0, max_workers=16, header_timeout=5, io_timeout=10,
                 scan_limits=None):
        self.roots, self.site_fd = roots, site_fd
        self.scan_limits = scan_limits or ScanLimits()
        self.media = {}
        self.media_lock = threading.Lock()
        self.catalog_lock = threading.Lock()
        self.slots = threading.BoundedSemaphore(max_workers)
        self.connections_lock = threading.Condition()
        self.connections = set()
        self.closing = threading.Event()
        self.header_timeout, self.io_timeout = header_timeout, io_timeout
        self.token = secrets.token_urlsafe(32)
        super().__init__(("127.0.0.1", port), Handler)

    def process_request(self, request, client_address):
        # A Ctrl-C between registering a connection and starting its worker
        # must not leave an unowned slot or release FDs while that worker runs.
        previous_mask = signal.pthread_sigmask(signal.SIG_BLOCK, {signal.SIGINT})
        try:
            self.start_request(request, client_address)
        finally:
            signal.pthread_sigmask(signal.SIG_SETMASK, previous_mask)

    def start_request(self, request, client_address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        with self.connections_lock:
            if self.closing.is_set():
                self.slots.release()
                self.shutdown_request(request)
                return
            self.connections.add(request)
        try:
            threading.Thread(target=self.process_request_thread, args=(request, client_address)).start()
        except BaseException:
            try:
                self.shutdown_request(request)
            finally:
                with self.connections_lock:
                    self.connections.discard(request)
                    self.slots.release()
                    self.connections_lock.notify_all()
            raise

    def process_request_thread(self, request, client_address):
        try:
            self.finish_request(request, client_address)
        except Exception:
            self.handle_error(request, client_address)
        finally:
            try:
                self.shutdown_request(request)
            finally:
                with self.connections_lock:
                    self.connections.discard(request)
                    self.slots.release()
                    self.connections_lock.notify_all()

    def server_close(self):
        with self.connections_lock:
            self.closing.set()
            for connection in self.connections:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
        super().server_close()
        # Workers may still be scanning; the caller must keep root FDs open until they finish.
        with self.connections_lock:
            while self.connections:
                self.connections_lock.wait()

    def handle_error(self, request, client_address):
        pass


class Handler(BaseHTTPRequestHandler):
    def setup(self):
        super().setup()
        self.rfile.close()
        self.rfile = io.BufferedReader(HeaderReader(self.connection, self.server.header_timeout,
                                                   self.server.io_timeout))

    def parse_request(self):
        try:
            return super().parse_request()
        finally:
            self.rfile.raw.finish_headers()

    def log_message(self, format, *args):
        pass

    def headers_for(self, status, content_type, length, extra=()):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(length))
        self.send_header("Referrer-Policy", "no-referrer")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Cross-Origin-Resource-Policy", "same-origin")
        self.send_header("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; media-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'")
        for key, value in extra:
            self.send_header(key, value)
        self.end_headers()

    def fail(self, status=404, extra=()):
        self.headers_for(status, "text/plain", 0, extra)

    def do_HEAD(self):
        self.respond(False)

    def do_GET(self):
        self.respond(True)

    def respond(self, body):
        try:
            if self.headers.get_all("Host") != [f"127.0.0.1:{self.server.server_port}"]:
                return self.fail()
            url = urlsplit(self.path)
            prefix = "/" + self.server.token + "/"
            if url.scheme or url.netloc or url.fragment or url.query or not url.path.startswith(prefix):
                return self.fail()
            relative = unquote(url.path[len(prefix):], errors="strict") or "index.html"
            parts(relative)
            if relative == "catalog.json":
                if not self.server.catalog_lock.acquire(blocking=False):
                    return self.fail(503, [("Retry-After", "1")])
                try:
                    if self.server.closing.is_set():
                        return
                    try:
                        budget = ScanBudget(self.server.scan_limits, self.server.closing)
                        catalog, media = local_catalog(self.server.roots, budget)
                        budget.check()
                        data = json.dumps(catalog, ensure_ascii=True).encode("utf-8")
                        budget.check()
                    except BaseException:
                        with self.server.media_lock:
                            self.server.media = {}
                        raise
                    with self.server.media_lock:
                        self.server.media = media
                finally:
                    self.server.catalog_lock.release()
                self.headers_for(200, "application/json; charset=utf-8", len(data))
                if body:
                    self.wfile.write(data)
                return
            if relative.startswith("media/"):
                clip_id = relative.removeprefix("media/")
                if not CLIP_ID.fullmatch(clip_id):
                    return self.fail()
                with self.server.media_lock:
                    media = self.server.media.get(clip_id)
                if media is None:
                    return self.fail()
                root_fd, name, stamp = media
                content_type = "video/mp4"
            else:
                assets = {"index.html": "text/html; charset=utf-8", "app.mjs": "text/javascript; charset=utf-8",
                          "lib.mjs": "text/javascript; charset=utf-8", "style.css": "text/css; charset=utf-8"}
                if relative not in assets:
                    return self.fail()
                root_fd, name, stamp = self.server.site_fd, relative, None
                content_type = assets[relative]
            with open_file(root_fd, name) as stream:
                info = os.fstat(stream.fileno())
                if stamp is not None and fingerprint(info) != stamp:
                    return self.fail(409)
                size = info.st_size
                start, end, status = 0, size - 1, 200
                extra = [("Accept-Ranges", "bytes")]
                ranges = self.headers.get_all("Range", [])
                if ranges:
                    match = re.fullmatch(r"bytes=([0-9]*)-([0-9]*)", ranges[0]) if len(ranges) == 1 else None
                    if not match or not any(match.groups()) or size == 0:
                        return self.fail(416, [("Content-Range", f"bytes */{size}")])
                    left, right = match.groups()
                    # Bound conversion before int() to handle arbitrarily long valid numbers.
                    def bounded(value):
                        value = value.lstrip("0") or "0"
                        return size + 1 if len(value) > len(str(size)) else min(int(value), size + 1)

                    if not left:
                        start = max(0, size - bounded(right))
                    else:
                        start = bounded(left)
                        if right:
                            end = min(end, bounded(right))
                    if start >= size or start > end:
                        return self.fail(416, [("Content-Range", f"bytes */{size}")])
                    status = 206
                    extra.append(("Content-Range", f"bytes {start}-{end}/{size}"))
                length = max(0, end - start + 1)
                self.headers_for(status, content_type, length, extra)
                if body:
                    stream.seek(start)
                    while length:
                        chunk = stream.read(min(length, 1024 * 1024))
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                        length -= len(chunk)
        except (OSError, ValueError, UnicodeError, RecursionError):
            self.fail()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=DEFAULT_ROOT, help="existing local video library")
    parser.add_argument("--draft-root", type=Path, help="optional separate review-draft root")
    parser.add_argument("--port", type=int, default=0)
    args = parser.parse_args()
    fds = []
    try:
        roots = {}
        for namespace, path in (("library", args.root), ("draft", args.draft_root)):
            if path is not None:
                fd = os.open(path, FLAGS | os.O_DIRECTORY)
                fds.append(fd)
                roots[namespace] = fd
        if "draft" in roots:
            if (directory_contains(roots["library"], roots["draft"])
                    or directory_contains(roots["draft"], roots["library"])):
                raise ValueError("library and draft roots must be separate and non-overlapping")
        site_fd = os.open(SITE, FLAGS | os.O_DIRECTORY)
        fds.append(site_fd)
        with ReviewServer(roots, site_fd, args.port) as server:
            print(f"http://127.0.0.1:{server.server_port}/{server.token}/", flush=True)
            try:
                server.serve_forever()
            except KeyboardInterrupt:
                pass
    except (OSError, ValueError) as exc:
        parser.exit(1, f"error: {exc}\n")
    finally:
        for fd in fds:
            os.close(fd)


if __name__ == "__main__":
    main()
