"""Pinned-source, offline native build recipe for x86-64 GNU/Linux."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
from urllib.parse import urlparse

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / 'tools/sdk'))
from native_inputs import host_glibc, locked_sources, source_directory


def verify_environment(recipes, with_dependencies=False):
    version = host_glibc()
    libraries = 'pinned sources' if with_dependencies else 'host development libraries'
    return (f'glibc\t{version}\tx86_64\n'
            f'Native dependencies: {libraries}; host libc/graphics are platform inputs.\n').encode()


def compile_payload(work, cache, recipes, with_dependencies=False, jobs=4):
    inventory = verify_environment(recipes, with_dependencies)
    if not work.is_absolute() or not cache.is_absolute():
        raise ValueError("build work and source cache paths must be absolute")
    # The caller verifies every archive first. A fresh tree prevents build
    # configuration or untracked source files surviving from previous attempts.
    work.mkdir(parents=True, exist_ok=False)
    sources, build, private = work / "sources", work / "build", work / "private"
    sources.mkdir(); build.mkdir()
    locked = locked_sources(recipes.parents[2])
    for source in locked.values():
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

    for project, patch in [("xwayland", "0001-xwayland-private-helpers.patch"),
                           ("xkbcomp", "0002-xkbcomp-parent-lifetime.patch")]:
        run("patch", "--batch", "--forward", "--fuzz=0", "-p1", "-i", recipes.parents[1] / "patches/xwayland" / patch,
            cwd=sources / source_directory(locked[project]))

    def meson(name, source, flags=(), target=None, install=False):
        directory = build / name
        run("meson", "setup", directory, sources / source_directory(locked[source]), "--prefix=" + str(private),
            "--libdir=lib", "--buildtype=release", "--wrap-mode=nodownload", *flags)
        run("meson", "compile", "-C", directory, "-j", str(jobs), *([target] if target else []))
        if install:
            run("meson", "install", "-C", directory, "--no-rebuild")

    meson("xorgproto", "xorgproto", install=True)
    meson("protocols", "wayland-protocols", ["-Dtests=false"], install=True)
    if with_dependencies:
        from dependencies import compile_libraries
        compile_libraries(entries, sources, build, private, run, jobs)
        (work / 'dependencies.lock.toml').write_bytes((recipes / 'dependencies.lock.toml').read_bytes())
    font = build / "font"
    font.mkdir()
    run(sources / source_directory(locked['libXfont2']) / 'configure', "--prefix=" + str(private), "--disable-static",
        "--enable-builtins", "--disable-fc", "--disable-devel-docs", cwd=font)
    run("make", "-j" + str(jobs), cwd=font)
    run("make", "install", cwd=font)
    meson("xkbcomp", "xkbcomp", ["-Dxkb-config-root=/nonexistent/telorgon-xkb"], target="xkbcomp")
    meson("keyboard", "xkeyboard-config", ["-Dnls=false"], install=True)
    meson("xwayland", "xwayland", [
        "-Dxvfb=false", "-Dglamor=true", "-Dglx=true", "-Ddri3=true", "-Ddrm=true",
        "-Dmitshm=true", "-Dxinerama=true", "-Dxv=true", "-Dxres=true",
        "-Dxdmcp=false", "-Dxdm-auth-1=false", "-Dsecure-rpc=false", "-Dlisten_tcp=false",
        "-Dlibdecor=false", "-Dxwayland_ei=false", "-Dsystemd_notify=false", "-Dxselinux=false",
        "-Ddocs=false", "-Ddevel-docs=false", "-Ddocs-pdf=false", "-Dsha1=libmd",
        "-Ddefault_font_path=built-ins", "-Dxkb_dir=/nonexistent/telorgon-xkb",
        "-Dxkb_bin_dir=/nonexistent/telorgon-bin", "-Dxkb_output_dir=/nonexistent/telorgon-xkm",
    ], target="Xwayland")
    (work / "build-packages.tsv").write_bytes(inventory)
