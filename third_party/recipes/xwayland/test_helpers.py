#!/usr/bin/env python3
"""Compile-only/helper tests in the isolated build root; never starts Xwayland.

Tests the exact patched keymap-launch functions in a small C harness, together
with the real built xkbcomp and selected keyboard data. These are not X11 display
or compositor integration tests.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

REPOSITORY = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY / 'tools/sdk'))
from native_inputs import locked_sources, source_directory


def keymap(symbols):
    return ('xkb_keymap {\n'
            ' xkb_keycodes { include "evdev+aliases(qwerty)" };\n'
            ' xkb_types { include "complete" };\n'
            ' xkb_compatibility { include "complete" };\n'
            f' xkb_symbols {{ include "pc+{symbols}+inet(evdev)" }};\n'
            ' xkb_geometry { include "pc(pc105)" };\n};\n').encode()


def run_tests(build_work, stage, work):
    relocated = work / "relocated space;$(touch forbidden-marker)"
    if relocated.exists():
        raise ValueError("test relocation directory already exists; use a fresh test work directory")
    shutil.copytree(stage, relocated)
    scratch = work / "private xkm;literal"
    scratch.mkdir(mode=0o700)
    xwayland = source_directory(locked_sources(REPOSITORY)['xwayland'])
    source = (build_work / 'sources' / xwayland / 'xkb/ddxLoad.c').read_text()
    start = source.index("static void\nOutputDirectory(")
    end = source.index("/**\n * Callback invoked", start)
    directory_function = source[start:end]
    start = source.index("/* The keymap callback cannot grow")
    end = source.index("typedef struct {", start)
    launch_functions = source[start:end]
    harness = r'''
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <signal.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>
static const char *XkbBaseDirectory;
static const char *display = "77";
static void FatalError(const char *message) { (void)message; exit(2); }
#define xnfstrdup strdup
typedef void (*xkbcomp_buffer_callback)(FILE *, void *);
'''
    harness += directory_function + launch_functions
    harness += r'''
static void input_callback(FILE *out, void *unused)
{
    char bytes[8192]; size_t n;
    (void)unused;
    while ((n = fread(bytes, 1, sizeof(bytes), stdin)) != 0)
        if (fwrite(bytes, 1, n, out) != n) break;
}
int main(int argc, char **argv)
{
    char *result;
    if (argc != 4) return 3;
    setenv("TELORGON_XKBCOMP", argv[1], 1);
    setenv("TELORGON_XKB_OUTPUT_DIR", argv[3], 1);
    XkbBaseDirectory = argv[2];
    result = RunXkbComp(input_callback, NULL);
    if (!result) return 1;
    free(result); return 0;
}
'''
    cfile, binary = work / "keymap-harness.c", work / "keymap-harness"
    cfile.write_text(harness)
    subprocess.run(["cc", "-O2", "-Wall", "-Wextra", "-Werror", str(cfile), "-o", str(binary)], check=True)
    compiler, data = relocated / "bin/xkbcomp", relocated / "share/X11/xkb"
    env = dict(os.environ)
    env.pop("LD_LIBRARY_PATH", None)
    env.pop("LD_PRELOAD", None)
    argv = [str(binary), str(compiler), str(data), str(scratch)]
    for symbols in ["us", "de", "fr", "us(dvorak)", "us+ru:2+group(alt_shift_toggle)"]:
        result = subprocess.run(argv, input=keymap(symbols), cwd=work, env=env, capture_output=True, timeout=8)
        if result.returncode != 0:
            raise AssertionError(f"keymap compile failed for {symbols}: status {result.returncode}")
        output = scratch / "server-77.xkm"
        assert output.stat().st_size > 1000, symbols
        output.unlink()
        print(f"PASS private relocated keymap: {symbols}")
    assert not (work / "forbidden-marker").exists()
    # Mismatched expected parent must be rejected before parsing compiler input.
    wrong_parent = dict(env, TELORGON_XKBCOMP_PARENT_PID="2147483647")
    result = subprocess.run([str(compiler), "-version"], env=wrong_parent, capture_output=True)
    assert result.returncode == 125
    print("PASS expected-parent identity mismatch")
    too_large = subprocess.run(argv, input=b"x" * (4 * 1024 * 1024 + 1), cwd=work,
                               env=env, capture_output=True, timeout=8)
    assert too_large.returncode == 1
    print("PASS bounded keymap input")
    stall_c, stall = work / "stall.c", work / "stall"
    stall_c.write_text("#include <unistd.h>\n#include <signal.h>\n#include <sys/prctl.h>\n"
                       "int main(void) { pid_t p=getppid(); prctl(PR_SET_PDEATHSIG,SIGKILL); "
                       "if(getppid()!=p) return 1; for (;;) pause(); }\n")
    subprocess.run(["cc", str(stall_c), "-o", str(stall)], check=True)
    started = time.monotonic()
    timeout = subprocess.run([str(binary), str(stall), str(data), str(scratch)],
                             input=keymap("us"), env=env, capture_output=True, timeout=8)
    assert timeout.returncode == 1 and 4.9 <= time.monotonic()-started < 7.5
    print("PASS bounded helper timeout and reap")
    bad_scratch = work / "scratch-symlink"
    bad_scratch.symlink_to(scratch)
    rejected = subprocess.run([str(binary), str(compiler), str(data), str(bad_scratch)],
                              input=keymap("us"), env=env, capture_output=True, timeout=8)
    assert rejected.returncode == 2
    print("PASS symlink scratch rejection")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--work", required=True, type=Path)
    parser.add_argument("--stage", required=True, type=Path)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="helper-tests-", dir=args.work) as test_root:
        run_tests(args.work, args.stage, Path(test_root))
