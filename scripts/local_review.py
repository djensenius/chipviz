#!/usr/bin/env python3
"""Opt-in, read-only loopback video review gallery."""

import argparse
import errno
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import stat
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


def digest_file(root_fd, relative, namespace):
    with open_file(root_fd, relative) as stream:
        before = fingerprint(os.fstat(stream.fileno()))
        digest = hashlib.sha256()
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
        if before != fingerprint(os.fstat(stream.fileno())):
            raise ValueError("file changed while hashing")
    raw = digest.digest()
    identity = namespace.encode() + b"\0" + relative.encode("utf-8") + b"\0" + raw
    return "clip-" + hashlib.sha256(identity).hexdigest()[:20], raw.hex(), before


def scan(root_fd, prefix=""):
    """Walk opened directories, never following symlinks, and yield MP4 names."""
    with os.scandir(root_fd) as entries:
        names = sorted(entry.name for entry in entries)
    for name in names:
        relative = prefix + name
        parts(relative)
        info = os.stat(name, dir_fd=root_fd, follow_symlinks=False)
        if stat.S_ISLNK(info.st_mode):
            continue
        if stat.S_ISDIR(info.st_mode):
            fd = os.open(name, FLAGS | os.O_DIRECTORY, dir_fd=root_fd)
            try:
                yield from scan(fd, relative + "/")
            finally:
                os.close(fd)
        elif stat.S_ISREG(info.st_mode) and name.lower().endswith(".mp4"):
            yield relative


def read_small(root_fd, relative):
    with open_file(root_fd, relative) as stream:
        before = fingerprint(os.fstat(stream.fileno()))
        data = stream.read(1024 * 1024 + 1)
        if len(data) > 1024 * 1024 or before != fingerprint(os.fstat(stream.fileno())):
            raise ValueError("invalid or changing metadata")
    return data.decode("utf-8")


def source_revision(root_fd, relative):
    names = parts(relative)
    for length in range(len(names) - 1, -1, -1):
        for filename in ("README.md", "README.txt"):
            try:
                text = read_small(root_fd, "/".join(names[:length] + [filename]))
            except OSError as exc:
                if exc.errno == errno.ENOENT:
                    continue
                raise
            matches = re.findall(
                r"(?im)^[ \t]*(?:[-*][ \t]*)?(?:source revision|source integration commit|revision)"
                r"[ \t]*:[ \t]*(?:`([0-9a-f]{40})`|([0-9a-f]{40}))(?=[.\s]|$)", text
            )
            revisions = {(first or second).lower() for first, second in matches}
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


def draft_revision(root_fd, relative):
    metadata = json.loads(read_small(root_fd, relative.rsplit("/", 1)[0] + "/provenance.json"),
                          object_pairs_hook=unique_object, parse_constant=invalid_constant)
    if not isinstance(metadata, dict) or not isinstance(metadata.get("source_commit"), str):
        raise ValueError("missing draft source commit")
    revision = metadata["source_commit"]
    if not REVISION.fullmatch(revision):
        raise ValueError("invalid draft source commit")
    return revision


def entry(root_fd, relative, namespace):
    clip_id, sha, stamp = digest_file(root_fd, relative, namespace)
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
        revision = draft_revision(root_fd, relative)
    elif len(names) >= 4 and names[0].lower() == "modern":
        code = names[1].lower()
        platform = PLATFORMS.get(code, names[1])
        family = FAMILIES.get(names[2].lower(), names[2])
        variant = names[3] if len(names) >= 5 else ("Original" if family == "Pulse Tunnel" else "Unknown")
        if code == "pi5":
            capture = "Pi 5 profile simulated on Mac GPU; not physical Pi hardware"
        elif code == "m1":
            capture = "M1 profile on Mac GPU"
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
        capture = "Emulated retro recording; not hardware validation"
    if not match:
        revision = source_revision(root_fd, relative)
    if namespace == "draft":
        notes.append("Review-only draft; not human-reviewed. Do not publish. Normal-speed human review and safety/flash assessment remain required.")
    return ({
        "id": clip_id, "sha256": sha, "title": title, "family": family,
        "platform": platform, "variant": variant, "scenario": scenario,
        "capture": capture, "source_revision": revision, "collection": namespace, "notes": notes,
    }, stamp)


def local_catalog(roots):
    clips, media = [], {}
    for namespace, root_fd in roots.items():
        for relative in scan(root_fd):
            clip, stamp = entry(root_fd, relative, namespace)
            if clip["id"] in media:
                raise ValueError("duplicate clip identity")
            media[clip["id"]] = (root_fd, relative, stamp)
            clips.append(clip)
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


class ReviewServer(HTTPServer):
    def __init__(self, roots, site_fd, port=0):
        self.roots, self.site_fd = roots, site_fd
        self.media = {}
        self.token = secrets.token_urlsafe(32)
        super().__init__(("127.0.0.1", port), Handler)

    def handle_error(self, request, client_address):
        pass


class Handler(BaseHTTPRequestHandler):
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
                self.server.media = {}
                catalog, media = local_catalog(self.server.roots)
                data = json.dumps(catalog, ensure_ascii=True).encode("utf-8")
                self.server.media = media
                self.headers_for(200, "application/json; charset=utf-8", len(data))
                if body:
                    self.wfile.write(data)
                return
            if relative.startswith("media/"):
                clip_id = relative.removeprefix("media/")
                if not CLIP_ID.fullmatch(clip_id) or clip_id not in self.server.media:
                    return self.fail()
                root_fd, name, stamp = self.server.media[clip_id]
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
