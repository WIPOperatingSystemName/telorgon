#!/usr/bin/env python3
"""Offline source verification, preflight and isolated-root payload compilation.

Never downloads or installs software. --compile requires a prepared glibc 2.39
build root and a fresh work directory. Compilation does not qualify a release.
"""
import argparse
import hashlib
import sys
from pathlib import Path
import shutil
import subprocess
import tomllib
from urllib.parse import urlparse


def verify_sources(root, lock):
    failures = []
    for source in lock["source"]:
        path = root / Path(urlparse(source["url"]).path).name
        try:
            with path.open("rb") as stream:
                actual = hashlib.file_digest(stream, "sha256").hexdigest()
            if actual != source["sha256"]:
                failures.append(f"digest mismatch: {path}")
        except OSError as error:
            failures.append(f"cannot read {path}: {error.strerror}")
    return failures


def preflight():
    failures = []
    for program in ("meson", "ninja", "make", "patch", "cc", "pkg-config", "wayland-scanner", "readelf", "patchelf"):
        if not shutil.which(program):
            failures.append(f"missing build tool: {program}")
    if not shutil.which("bison") and not shutil.which("byacc"):
        failures.append("missing native parser generator: bison or byacc")
    if shutil.which("pkg-config"):
        # Protocol XML/headers and libXfont2 are compiled from locked sources by
        # the recipe; do not require those finished inputs to be installed first.
        for dependency in ("wayland-client >= 1.21", "wayland-scanner >= 1.20", "libdrm >= 2.4.116",
                           "xkbfile", "x11", "xcb", "pixman-1", "epoxy", "libxcvt", "xshmfence",
                           "libmd", "fontenc", "freetype2", "zlib", "dri"):
            if subprocess.run(["pkg-config", "--exists", dependency], check=False).returncode:
                failures.append(f"missing build dependency: {dependency}")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-cache", type=Path, help="verify all locked source archives in this directory")
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--compile", type=Path, metavar="FRESH_WORK_DIRECTORY")
    args = parser.parse_args()
    if not args.source_cache and not args.preflight:
        parser.error("select --source-cache and/or --preflight")
    if args.compile and not args.source_cache:
        parser.error("--compile requires --source-cache")
    repository = Path(__file__).resolve().parents[2]
    recipes = repository / "third_party/recipes/xwayland"
    lock = tomllib.loads((repository / "third_party/sources.lock.toml").read_text())
    failures = preflight() if args.preflight else []
    if args.source_cache:
        failures.extend(verify_sources(args.source_cache, lock))
    for failure in failures:
        print(failure)
    if not failures and args.compile:
        sys.path.insert(0, str(recipes))
        from recipe import compile_payload
        compile_payload(args.compile, args.source_cache, recipes)
    elif not failures:
        print("Requested prerequisite checks passed; release build and qualification remain outstanding.")
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    main()
