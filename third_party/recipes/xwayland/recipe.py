"""Pinned-source, offline native build recipe for an isolated glibc 2.39 root."""
import ctypes
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tomllib
from urllib.parse import urlparse


def verify_environment(recipes, with_dependencies=False):
    libc = ctypes.CDLL(None)
    libc.gnu_get_libc_version.restype = ctypes.c_char_p
    if platform.machine() != "x86_64" or libc.gnu_get_libc_version() != b"2.39":
        raise ValueError("build in the isolated x86-64/glibc 2.39 root")
    if with_dependencies:
        return b"Native dependencies compiled from dependencies.lock.toml; host libc/graphics are platform inputs.\n"
    inventory = subprocess.check_output(
        ["dpkg-query", "-W", "-f=${binary:Package}\t${Version}\t${Architecture}\n"])
    if not with_dependencies and inventory != (recipes / "build-packages.lock.tsv").read_bytes():
        raise ValueError("isolated build package inventory differs from build-packages.lock.tsv")
    return inventory


def compile_payload(work, cache, recipes, with_dependencies=False, jobs=4):
    inventory = verify_environment(recipes, with_dependencies)
    if not work.is_absolute() or not cache.is_absolute():
        raise ValueError("build work and source cache paths must be absolute")
    # The caller verifies every archive first. A fresh tree prevents build
    # configuration or untracked source files surviving from previous attempts.
    work.mkdir(parents=True, exist_ok=False)
    sources, build, private = work / "sources", work / "build", work / "private"
    sources.mkdir(); build.mkdir()
    lock = tomllib.loads((recipes.parents[1] / "sources.lock.toml").read_text())
    for source in lock["source"]:
        archive = cache / Path(urlparse(source["url"]).path).name
        with tarfile.open(archive) as stream:
            stream.extractall(sources, filter="data")
    entries = []
    if with_dependencies:
        from dependencies import load, extract
        entries = load(recipes)
        extract(cache, sources, entries)
    env = dict(os.environ, CFLAGS="-O2 -march=x86-64 -mtune=generic",
               CXXFLAGS="-O2 -march=x86-64 -mtune=generic",
               PKG_CONFIG_PATH=str(private / "lib/pkgconfig") + ":" + str(private / "share/pkgconfig"))
    env.pop("LD_LIBRARY_PATH", None)
    env.pop("LD_PRELOAD", None)
    if with_dependencies:
        env.update(
            PATH=str(private / 'bin') + os.pathsep + env['PATH'],
            CPPFLAGS='-I' + str(private / 'include'),
            LDFLAGS='-L' + str(private / 'lib') + ' -Wl,-rpath,' + str(private / 'lib'),
            CMAKE_PREFIX_PATH=str(private),
            PYTHONPATH=str(private / f'lib/python{sys.version_info.major}.{sys.version_info.minor}/site-packages'),
        )
    commands = []

    def run(*args, cwd=None):
        command = list(map(str, args))
        commands.append(dict(argv=command, cwd=str(cwd or work)))
        (work / "build-commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        print("RUN", *command, flush=True)
        subprocess.run(command, cwd=cwd or work, env=env, check=True)

    for project, patch in [("xwayland-24.1.13", "0001-xwayland-private-helpers.patch"),
                           ("xkbcomp-1.5.0", "0002-xkbcomp-parent-lifetime.patch")]:
        run("patch", "--batch", "--forward", "--fuzz=0", "-p1", "-i", recipes.parents[1] / "patches/xwayland" / patch,
            cwd=sources / project)

    def meson(name, source, flags=(), target=None, install=False):
        directory = build / name
        run("meson", "setup", directory, sources / source, "--prefix=" + str(private),
            "--libdir=lib", "--buildtype=release", "--wrap-mode=nodownload", *flags)
        run("meson", "compile", "-C", directory, "-j", str(jobs), *([target] if target else []))
        if install:
            run("meson", "install", "-C", directory, "--no-rebuild")

    meson("xorgproto", "xorgproto-2024.1", install=True)
    meson("protocols", "wayland-protocols-1.47", ["-Dtests=false"], install=True)
    if with_dependencies:
        from dependencies import compile_libraries
        compile_libraries(entries, sources, build, private, run, jobs)
        (work / 'dependencies.lock.toml').write_bytes((recipes / 'dependencies.lock.toml').read_bytes())
    font = build / "font"
    font.mkdir()
    run(sources / "libXfont2-2.0.8/configure", "--prefix=" + str(private), "--disable-static",
        "--enable-builtins", "--disable-fc", "--disable-devel-docs", cwd=font)
    run("make", "-j" + str(jobs), cwd=font)
    run("make", "install", cwd=font)
    meson("xkbcomp", "xkbcomp-1.5.0", ["-Dxkb-config-root=/nonexistent/telorgon-xkb"], target="xkbcomp")
    meson("keyboard", "xkeyboard-config-2.46", ["-Dnls=false"], install=True)
    meson("xwayland", "xwayland-24.1.13", [
        "-Dxvfb=false", "-Dglamor=true", "-Dglx=true", "-Ddri3=true", "-Ddrm=true",
        "-Dmitshm=true", "-Dxinerama=true", "-Dxv=true", "-Dxres=true",
        "-Dxdmcp=false", "-Dxdm-auth-1=false", "-Dsecure-rpc=false", "-Dlisten_tcp=false",
        "-Dlibdecor=false", "-Dxwayland_ei=false", "-Dsystemd_notify=false", "-Dxselinux=false",
        "-Ddocs=false", "-Ddevel-docs=false", "-Ddocs-pdf=false", "-Dsha1=libmd",
        "-Ddefault_font_path=built-ins", "-Dxkb_dir=/nonexistent/telorgon-xkb",
        "-Dxkb_bin_dir=/nonexistent/telorgon-bin", "-Dxkb_output_dir=/nonexistent/telorgon-xkm",
    ], target="Xwayland")
    (work / "build-packages.tsv").write_bytes(inventory)
