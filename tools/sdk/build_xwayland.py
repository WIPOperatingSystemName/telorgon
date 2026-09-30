#!/usr/bin/env python3
"""Offline source verification, preflight and native payload compilation.

--fetch downloads checksum-pinned sources. --with-dependencies builds private
libraries as well as Xwayland. Build tools, GNU libc and host graphics development
interfaces must be available. No system packages are installed. --compile requires
a fresh work directory. Compilation does not qualify a release.
"""
import argparse
import sys
from pathlib import Path
import shutil
import subprocess
import tomllib

from native_inputs import locked_sources


def preflight(with_dependencies=False, environment=None):
    failures = []
    programs = ("meson", "ninja", "make", "patch", "cc", "pkg-config", "wayland-scanner", "readelf", "patchelf")
    if with_dependencies:
        programs = (*[p for p in programs if p != "wayland-scanner"], "cmake", "c++", "perl")
    for program in programs:
        if not shutil.which(program):
            failures.append(f"missing build tool: {program}")
    if not shutil.which("bison") and not shutil.which("byacc"):
        failures.append("missing native parser generator: bison or byacc")
    if shutil.which("pkg-config"):
        # Protocol XML/headers and libXfont2 are compiled from locked sources by
        # the recipe; do not require those finished inputs to be installed first.
        dependencies = ("wayland-client >= 1.21", "wayland-scanner >= 1.20", "libdrm >= 2.4.116",
                           "xkbfile", "x11", "xcb", "pixman-1", "epoxy", "libxcvt", "xshmfence",
                           "libmd", "fontenc", "freetype2", "zlib", "dri")
        if with_dependencies:
            dependencies = ("libdrm >= 2.4.116", "gbm", "egl", "gl", "dri")
        for dependency in dependencies:
            if subprocess.run(["pkg-config", "--exists", dependency], env=environment,
                              check=False).returncode:
                failures.append(f"missing build dependency: {dependency}")
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-cache", type=Path, help="verify all locked source archives in this directory")
    parser.add_argument("--preflight", action="store_true")
    parser.add_argument("--compile", type=Path, metavar="FRESH_WORK_DIRECTORY")
    parser.add_argument('--fetch', action='store_true', help='download missing pinned sources over HTTPS')
    parser.add_argument('--with-dependencies', action='store_true', help='build pinned native libraries into the private prefix')
    parser.add_argument('--jobs', type=int, default=4)
    parser.add_argument('--output', type=Path, help='stage and package the completed build to this payload path')
    args = parser.parse_args()
    if not 1 <= args.jobs <= 64:
        parser.error('--jobs must be between 1 and 64')
    if args.fetch and not args.source_cache:
        parser.error('--fetch requires --source-cache')
    if args.output and not args.compile:
        parser.error('--output requires --compile')
    if not args.source_cache and not args.preflight:
        parser.error("select --source-cache and/or --preflight")
    if args.compile and not args.source_cache:
        parser.error("--compile requires --source-cache")
    repository = Path(__file__).resolve().parents[2]
    recipes = repository / "third_party/recipes/xwayland"
    failures = preflight(args.with_dependencies) if args.preflight else []
    sources = list(locked_sources(repository).values())
    if args.with_dependencies:
        sources.extend(tomllib.loads((recipes / 'dependencies.lock.toml').read_text())['source'])
    if args.source_cache:
        from source_cache import prepare
        prepare(args.source_cache, sources, fetch=args.fetch)
    for failure in failures:
        print(failure)
    if not failures and args.compile:
        sys.path.insert(0, str(recipes))
        from recipe import compile_payload
        compile_payload(args.compile, args.source_cache, recipes, args.with_dependencies, args.jobs)
        if args.output:
            packaging = repository / 'packaging/linux/xwayland'
            subprocess.run([sys.executable, str(packaging / 'stage.py'), '--work',
                            str(args.compile), '--stage', str(args.compile / 'stage')], check=True)
            subprocess.run([sys.executable, str(packaging / 'pack.py'), '--stage',
                            str(args.compile / 'stage'), '--components', str(args.compile / 'components.json'),
                            '--output', str(args.output)], check=True)
    elif not failures:
        print("Requested source/preflight checks passed; no compilation performed.")
    raise SystemExit(1 if failures else 0)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
