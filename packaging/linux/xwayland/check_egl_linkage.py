#!/usr/bin/env python3
"""Loader-only compatibility check; never starts Xwayland or initializes a GPU/display.

Run on the deployment host, not just the older isolated build root. Loads the helper's
direct dependencies with its staged RUNPATH choices before linking the selected EGL
vendor. This tests symbol resolution, not EGL initialization, rendering or every
possible LD_PRELOAD/LD_LIBRARY_PATH configuration.
"""
import argparse
import ctypes
import os
from pathlib import Path
import re
import subprocess
import sys


def probe(stage, vendor):
    dynamic = subprocess.check_output(["readelf", "-d", str(stage / "bin/Xwayland")], text=True)
    needed = re.findall(r"\(NEEDED\).*\[(.*?)\]", dynamic)
    if not needed:
        raise ValueError("Xwayland has no dynamic dependency list")
    loaded = []
    for soname in needed:
        private = stage / "lib" / soname
        name = str(private) if private.is_file() else soname
        loaded.append(ctypes.CDLL(name, mode=os.RTLD_NOW | os.RTLD_GLOBAL))
    # NOW resolves function references too; no eglInitialize, device FD or display connection.
    loaded.append(ctypes.CDLL(vendor, mode=os.RTLD_NOW | os.RTLD_LOCAL))
    print(f"PASS: {vendor} links after {len(needed)} Xwayland dependencies ({stage})")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--vendor-library", required=True, help="For Mesa: libEGL_mesa.so.0")
    parser.add_argument("--probe", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.probe:
        try:
            probe(args.stage.resolve(), args.vendor_library)
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            print(f"FAIL: EGL vendor linkage: {error}", file=sys.stderr)
            return 1
        return 0
    # Always use a fresh process; SONAME reuse is exactly the failure under test.
    return subprocess.run([sys.executable, str(Path(__file__).resolve()), "--probe",
                           "--stage", str(args.stage.resolve()),
                           "--vendor-library", args.vendor_library], check=False).returncode


if __name__ == "__main__":
    sys.exit(main())
