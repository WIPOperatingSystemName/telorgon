#!/usr/bin/env python3
"""Prepare/use an offline private PipeWire native SDK. No download or system installation.

Prepare sources separately from the URL in third_party/recipes/pipewire/source.lock.toml.
Compile in the pinned native build root, using a NEW absolute work directory:
  python3 tools/sdk/build_pipewire.py --source-cache /cache --compile /work/pw
Use the resulting prefix for a scoped Cargo invocation (also supplies runtime paths):
  python3 tools/sdk/build_pipewire.py --prefix /work/pw/stage/opt/telorgon/pipewire -- cargo build --locked -p telorgon --no-default-features --features audio-linux
Without --compile, --source-cache only verifies the archive. No tests run automatically.
The existing PipeWire server, WirePlumber and portal services remain host facilities.
This minimal native profile omits D-Bus/RTKit scheduling escalation and hardware
plugins; devices are provided by the existing server. Native RT permission is used
when available. This profile is not yet build/runtime or release qualified.
"""
import argparse
from pathlib import Path
import subprocess
import sys


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source-cache', type=Path)
    parser.add_argument('--compile', type=Path, metavar='FRESH_WORK_DIRECTORY')
    parser.add_argument('--jobs', type=int, default=4)
    parser.add_argument('--prefix', type=Path, help='run command with a completed private prefix')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if not 1 <= args.jobs <= 64:
        parser.error('--jobs must be between 1 and 64')
    repository = Path(__file__).resolve().parents[2]
    recipe_dir = repository / 'third_party/recipes/pipewire'
    sys.path.insert(0, str(recipe_dir))
    from recipe import load_lock, verify_archive, compile_sdk, runtime_environment
    if args.prefix:
        if args.source_cache or args.compile:
            parser.error('--prefix cannot be combined with preparation')
        command = args.command[1:] if args.command[:1] == ['--'] else args.command
        if not command:
            parser.error('--prefix requires -- COMMAND [ARGUMENTS]')
        env = runtime_environment(args.prefix.resolve(strict=True), recipe_dir)
        return subprocess.run(command, env=env, check=False).returncode
    if args.command or not args.source_cache:
        parser.error('provide --source-cache or --prefix with a command')
    lock = load_lock(recipe_dir)
    archive = args.source_cache.resolve(strict=True) / lock['archive']
    verify_archive(archive, lock)
    if args.compile:
        if not args.compile.is_absolute():
            parser.error('--compile must be an absolute, fresh directory')
        private = compile_sdk(args.compile.resolve(), archive, recipe_dir, args.jobs)
        print('Private PipeWire SDK:', private)
    else:
        print('PipeWire archive matches the lock; no build performed.')
    return 0


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
