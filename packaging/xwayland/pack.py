#!/usr/bin/env python3
"""Pack an already staged, reviewed payload. No network or executable launches.

Input components JSON is an array of {name,version,input_sha256,license}.
Archive hashes establish integrity, not upstream signature verification or
runtime qualification. See README.md for the outstanding release gates.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import struct
import tempfile
import zlib

MAX_COMPRESSED = 64 * 1024 * 1024
MAX_EXPANDED = 256 * 1024 * 1024
MAX_ENTRIES = 16384
MAX_MANIFEST = 4 * 1024 * 1024


def valid_path(path):
    return bool(path) and len(path) <= 1024 and all(
        part not in ("", ".", "..") and len(part) <= 255
        and all(c in "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._+-" for c in part)
        for part in path.split("/")
    )


def digest(data):
    return hashlib.sha256(data).hexdigest()


def pack(stage, components):
    stage = Path(stage)
    if stage.is_symlink() or not stage.is_dir():
        raise ValueError("stage must be a real directory")
    if not isinstance(components, list) or not 1 <= len(components) <= 256:
        raise ValueError("invalid component inventory")
    names = set()
    for c in components:
        if set(c) != {"name", "version", "input_sha256", "license"} or not all(isinstance(v, str) for v in c.values()):
            raise ValueError("invalid component fields")
        if not valid_path(c["name"]) or c["name"] in names or not 1 <= len(c["version"]) <= 128 or not 1 <= len(c["license"]) <= 1024:
            raise ValueError("invalid or duplicate component")
        if len(c["input_sha256"]) != 64 or any(c not in "0123456789abcdef" for c in c["input_sha256"]):
            raise ValueError("invalid component source hash")
        names.add(c["name"])
    entries, chunks, expanded, offset = [], [], 0, 0
    # Walk explicitly: Path.rglob can silently omit directory symlinks.
    pending, paths, visited = [stage], [], 0
    while pending:
        parent = pending.pop()
        for path in sorted(parent.iterdir()):
            visited += 1
            if visited > MAX_ENTRIES:
                raise ValueError("too many staged entries (including directories)")
            relative = path.relative_to(stage).as_posix()
            if not valid_path(relative):
                raise ValueError(f"unsafe stage path: {relative}")
            info = path.lstat()
            if stat.S_ISDIR(info.st_mode):
                pending.append(path)
            elif stat.S_ISREG(info.st_mode) and info.st_nlink == 1:
                paths.append(path)
            else:
                raise ValueError(f"symlink, hardlink or special file in stage: {relative}")
            if len(paths) + len(pending) > MAX_ENTRIES:
                raise ValueError("too many staged entries")
    for path in sorted(paths):
        relative = path.relative_to(stage).as_posix()
        # The stage must be owned by the build process and quiescent while
        # packing. O_NOFOLLOW and fstat also reject leaf swaps to special files.
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
        with os.fdopen(fd, "rb") as source:
            info = os.fstat(source.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_size > MAX_EXPANDED - expanded:
                raise ValueError(f"invalid stage file or expanded bound exceeded: {relative}")
            raw = source.read(info.st_size + 1)
        if len(raw) != info.st_size:
            raise ValueError(f"stage changed during packing: {relative}")
        executable = relative.startswith("bin/")
        if executable and (len(raw) < 64 or raw[:6] != b"\x7fELF\x02\x01" or raw[18:20] != b"\x3e\x00"):
            raise ValueError(f"expected x86-64 ELF executable: {relative}")
        compressor = zlib.compressobj(9, zlib.DEFLATED, -15)
        compressed = compressor.compress(raw) + compressor.flush()
        entries.append(dict(path=relative, mode=0o500 if executable else 0o400,
                            offset=offset, compressed_size=len(compressed),
                            size=len(raw), sha256=digest(raw)))
        chunks.append(compressed)
        offset += len(compressed)
        expanded += len(raw)
        if offset > MAX_COMPRESSED:
            raise ValueError("compressed bound exceeded")
    names = {e["path"] for e in entries}
    if not {"bin/Xwayland", "bin/xkbcomp"} <= names or not any(p.startswith("share/X11/xkb/") for p in names) or not any(p.startswith("licenses/") for p in names):
        raise ValueError("stage lacks required executables, keyboard data or notices")
    manifest = dict(schema=1, target="x86_64-unknown-linux-gnu", minimum_glibc="2.39",
                    xwayland_version="24.1.13", components=sorted(components, key=lambda c: c["name"]), entries=entries)
    encoded = json.dumps(manifest, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()
    if len(encoded) > MAX_MANIFEST:
        raise ValueError("manifest bound exceeded")
    archive = b"TLXWP001" + struct.pack("<I", len(encoded)) + encoded + b"".join(chunks)
    if len(archive) > MAX_COMPRESSED:
        raise ValueError("archive bound exceeded")
    return archive


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", required=True, type=Path)
    parser.add_argument("--components", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    archive = pack(args.stage, json.loads(args.components.read_text()))
    # Publish atomically; never leave a truncated archive at the requested path.
    with tempfile.NamedTemporaryFile(dir=args.output.parent, delete=False) as out:
        temporary = Path(out.name)
        try:
            out.write(archive)
            out.flush()
            os.fsync(out.fileno())
            out.close()
            os.replace(temporary, args.output)
        finally:
            temporary.unlink(missing_ok=True)
    print(f"{digest(archive)}  {args.output} ({len(archive)} bytes)")


if __name__ == "__main__":
    main()
