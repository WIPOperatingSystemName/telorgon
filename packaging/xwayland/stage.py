#!/usr/bin/env python3
"""Stage a built payload inside the isolated build root, without launching it.

Reads DT_NEEDED rather than executing ldd. Every non-host SONAME must appear in
runtime-policy.toml. This establishes a static closure, not dlopen qualification.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


def output(*argv):
    return subprocess.check_output(list(map(str, argv)), text=True)


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def stage(work, destination, recipes):
    policy = tomllib.loads((recipes / "runtime-policy.toml").read_text())
    lock = tomllib.loads((recipes / "sources.lock.toml").read_text())
    if destination.exists():
        raise ValueError("stage must be a new directory")
    destination.mkdir(parents=True, mode=0o700)
    (destination / "bin").mkdir()
    (destination / "lib").mkdir()
    (destination / "licenses").mkdir()
    inventory, edges, seen = [], {}, set()
    for source in lock["source"]:
        inventory.append(dict(name=source["name"], version=source["version"],
                              input_sha256=source["sha256"], license="See licenses and source references"))
        directory = work / "sources" / (source["name"] + "-" + source["version"])
        candidates = [directory / name for name in ("COPYING", "COPYING.md", "LICENSE", "LICENSE.txt")]
        for notice in candidates:
            if notice.is_file():
                shutil.copyfile(notice, destination / "licenses" / (source["name"] + "-" + notice.name))
    for patch in sorted((recipes / "patches").glob("*.patch")):
        inventory.append(dict(name=patch.stem, version="1", input_sha256=sha(patch), license="GPL-3.0-or-later; upstream notices retained"))
        shutil.copyfile(patch, destination / "licenses" / patch.name)
    shutil.copyfile(recipes / "patches/LICENSE", destination / "licenses/Telorgon-GPL-3.0.txt")
    (destination / "licenses" / "sources.lock.toml").write_bytes((recipes / "sources.lock.toml").read_bytes())
    pending = []
    for name, source in [("Xwayland", work / "build/xwayland/hw/xwayland/Xwayland"),
                         ("xkbcomp", work / "build/xkbcomp/xkbcomp")]:
        target = destination / "bin" / name
        shutil.copyfile(source, target)
        target.chmod(0o700)
        pending.append(target)
    while pending:
        path = pending.pop()
        versions = [tuple(map(int, v.split("."))) for v in re.findall(r"GLIBC_(\d+(?:\.\d+)+)", output("readelf", "--version-info", path))]
        if any(v > (2, 39) for v in versions):
            raise ValueError(f"host glibc floor exceeded: {path.name}")
        needed = re.findall(r"\(NEEDED\).*\[(.*?)\]", output("readelf", "-d", path))
        edges[path.relative_to(destination).as_posix()] = dict(needed=needed, glibc_floor=".".join(map(str, max(versions))) if versions else None)
        for soname in needed:
            if soname in policy["host"] or soname in seen:
                continue
            if soname not in policy["private"]:
                raise ValueError(f"undeclared runtime library: {soname}")
            source = next((p / soname for p in [work / "private/lib", Path("/usr/lib/x86_64-linux-gnu")]
                           if (p / soname).is_file()), None)
            if source is None:
                raise ValueError(f"missing private runtime library: {soname}")
            target = destination / "lib" / soname
            shutil.copyfile(source, target)
            seen.add(soname)
            pending.append(target)
            if source.is_relative_to(work):
                continue  # Built libXfont2 provenance is already recorded above.
            resolved = source.resolve()
            record = output("dpkg-query", "-S", resolved).splitlines()[0]
            package = record.split(": ", 1)[0]
            version = output("dpkg-query", "-W", "-f=${Version}", package).strip()
            notice = Path("/usr/share/doc") / package.split(":")[0] / "copyright"
            if not notice.is_file():
                raise ValueError(f"missing package notice: {package}")
            shutil.copyfile(notice, destination / "licenses" / (soname + ".copyright"))
            inventory.append(dict(name=soname, version=version, input_sha256=sha(source), license="licenses/" + soname + ".copyright"))
    for relative in edges:
        path = destination / relative
        runpath = "$ORIGIN/../lib" if relative.startswith("bin/") else "$ORIGIN"
        subprocess.run(["patchelf", "--set-rpath", runpath, str(path)], check=True)
        actual = output("patchelf", "--print-rpath", path).strip()
        if actual != runpath:
            raise ValueError(f"RUNPATH relocation failed: {relative}")
    keyboard = destination / "share/X11/xkb"
    shutil.copytree(work / "private/share/xkeyboard-config-2", keyboard, symlinks=False)
    (destination / "licenses/static-closure.json").write_text(json.dumps(edges, indent=2, sort_keys=True) + "\n")
    (work / "components.json").write_text(json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    (work / "static-closure.json").write_text(json.dumps(edges, indent=2, sort_keys=True) + "\n")
    print(f"Staged {len(seen)} private libraries; dynamic loading remains unqualified.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--stage", type=Path, required=True)
    args = parser.parse_args()
    stage(args.work, args.stage, Path(__file__).resolve().parent)


if __name__ == "__main__":
    main()
