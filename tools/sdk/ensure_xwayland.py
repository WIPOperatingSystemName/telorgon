#!/usr/bin/env python3
"""Ensure the compositor's native payload exists, rebuilding only on a cache miss."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib

from source_cache import prepare

TARGET = 'x86_64-unknown-linux-gnu'
INPUTS = (
    'tools/sdk/ensure_xwayland.py', 'tools/sdk/build_xwayland.py',
    'tools/sdk/source_cache.py', 'third_party/sources.lock.toml',
    'third_party/recipes/xwayland', 'third_party/patches/xwayland',
    'packaging/linux/xwayland/stage.py', 'packaging/linux/xwayland/pack.py',
    'packaging/linux/xwayland/runtime-policy.toml',
    'crates/telorgon/src/integrations/x11/payload_format.rs',
)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def input_files(repository):
    for relative in INPUTS:
        path = repository / relative
        if path.is_dir():
            yield from sorted(p for p in path.rglob('*')
                              if p.is_file() and '__pycache__' not in p.parts and not p.name.startswith('test_'))
        else:
            yield path


def fingerprint(repository, target, build_root):
    records = [(str(path.relative_to(repository)), digest(path))
               for path in input_files(repository)]
    environment = {'target': target, 'root': str(build_root) if build_root else 'host'}
    if build_root:
        # Changes to the prepared native SDK invalidate binaries built against it.
        for relative in ('var/lib/dpkg/status', 'etc/os-release', 'usr/local/bin/cmake'):
            path = build_root / relative
            if path.exists():
                environment[relative] = digest(path)
    else:
        environment['libc'] = platform.libc_ver()
        environment['machine'] = platform.machine()
    return hashlib.sha256(json.dumps([records, environment], sort_keys=True).encode()).hexdigest()


def payload_header(path, target):
    if not path.is_file() or not 12 <= path.stat().st_size <= 64 * 1024 * 1024:
        raise ValueError('invalid Xwayland payload size')
    with path.open('rb') as stream:
        if stream.read(8) != b'TLXWP001':
            raise ValueError('invalid Xwayland payload signature')
        length = struct.unpack('<I', stream.read(4))[0]
        if length > 4 * 1024 * 1024:
            raise ValueError('invalid Xwayland manifest size')
        manifest = json.loads(stream.read(length))
    if not isinstance(manifest, dict):
        raise ValueError('invalid Xwayland manifest')
    if (manifest.get('schema'), manifest.get('target'), manifest.get('minimum_glibc')) != (1, target, '2.39'):
        raise ValueError('incompatible Xwayland payload')
    # Cargo performs the full archive validation before embedding these bytes.


def cached_payload(cache, key, target):
    payload = cache / 'xwayland.payload'
    try:
        marker = json.loads((cache / 'build.json').read_text())
        if marker['fingerprint'] != key or marker['sha256'] != digest(payload):
            return None
        payload_header(payload, target)
        return payload
    except (OSError, ValueError, KeyError, TypeError, struct.error):
        return None


def atomic_json(path, value):
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as stream:
        temporary = Path(stream.name)
        try:
            json.dump(value, stream, sort_keys=True)
            stream.flush()
            os.fsync(stream.fileno())
            stream.close()
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)


def ensure(repository, cache, target, build_root, builder):
    """Serialize concurrent Cargo users and publish only a completed successful build."""
    cache.mkdir(parents=True, exist_ok=True)
    with (cache / 'build.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        key = fingerprint(repository, target, build_root)
        hit = cached_payload(cache, key, target)
        if hit:
            return hit
        attempt = Path(tempfile.mkdtemp(prefix='build-', dir=cache))
        print(f'Telorgon: preparing Xwayland; build log: {attempt / "build.log"}', file=sys.stderr, flush=True)
        candidate = builder(repository, attempt, target, build_root)
        payload_header(candidate, target)
        if fingerprint(repository, target, build_root) != key:
            raise ValueError('Xwayland build inputs changed during compilation; retry the build')
        # Preserve the old working payload on build failure. The marker is published last;
        # an interruption between replacements becomes a cache miss on the next invocation.
        payload_hash = digest(candidate)
        candidate.replace(cache / 'xwayland.payload')
        atomic_json(cache / 'build.json', {'fingerprint': key, 'sha256': payload_hash})
        return cache / 'xwayland.payload'


def build(repository, attempt, target, build_root):
    sources = tomllib.loads((repository / 'third_party/sources.lock.toml').read_text())['source']
    sources += tomllib.loads((repository / 'third_party/recipes/xwayland/dependencies.lock.toml').read_text())['source']
    source_cache = repository / 'target/native-source-cache'
    # Downloads happen before entering the network-disabled build environment.
    with contextlib.redirect_stdout(sys.stderr):
        prepare(source_cache, sources, fetch=os.environ.get('CARGO_NET_OFFLINE') != 'true')
    native_repository = Path('/mnt/telorgon') if build_root else repository
    native_attempt = Path('/mnt/build') if build_root else attempt
    native_sources = Path('/mnt/sources') if build_root else source_cache
    arguments = ['--preflight', '--source-cache', str(native_sources), '--with-dependencies',
                 '--compile', str(native_attempt / 'work'), '--output', str(native_attempt / 'xwayland.payload')]
    jobs = os.environ.get('NUM_JOBS', '4')
    arguments += ['--jobs', str(min(64, max(1, int(jobs))))]
    environment = dict(os.environ)
    # Do not let application-specific flags or Cargo linker settings leak into the native SDK.
    for name in ('CC', 'CXX', 'AR', 'CFLAGS', 'CXXFLAGS', 'CPPFLAGS', 'LDFLAGS',
                 'LD_LIBRARY_PATH', 'LD_PRELOAD', 'PKG_CONFIG_PATH', 'PKG_CONFIG_LIBDIR',
                 'PKG_CONFIG_SYSROOT_DIR', 'CMAKE_PREFIX_PATH', 'PYTHONPATH', 'DESTDIR'):
        environment.pop(name, None)
    environment['PYTHONDONTWRITEBYTECODE'] = '1'
    if build_root:
        if not shutil.which('bwrap'):
            raise ValueError('Xwayland isolated builds require bwrap in PATH')
        # Fixed mount points work even when the checkout is under a read-only root.
        # Only this attempt is writable; the prepared root is never modified.
        command = ['bwrap', '--unshare-all', '--die-with-parent', '--uid', '0', '--gid', '0',
                   '--ro-bind', str(build_root), '/', '--proc', '/proc', '--dev', '/dev',
                   '--tmpfs', '/tmp', '--tmpfs', '/mnt',
                   '--ro-bind', str(repository), str(native_repository),
                   '--ro-bind', str(source_cache), str(native_sources),
                   '--bind', str(attempt), str(native_attempt), '--clearenv',
                   '--setenv', 'PATH', '/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin',
                   '--setenv', 'HOME', '/tmp', '--setenv', 'LC_ALL', 'C.UTF-8',
                   '--setenv', 'PYTHONDONTWRITEBYTECODE', '1', '--chdir', str(native_attempt),
                   'python3', str(native_repository / 'tools/sdk/build_xwayland.py'), *arguments]
    else:
        if platform.libc_ver() != ('glibc', '2.39'):
            raise ValueError('Xwayland requires a glibc 2.39 build environment. Set '
                             'TELORGON_XWAYLAND_BUILD_ROOT to a prepared root, or '
                             'TELORGON_XWAYLAND_PAYLOAD to an existing payload.')
        command = [sys.executable, str(repository / 'tools/sdk/build_xwayland.py'), *arguments]
    with (attempt / 'build.log').open('w') as log:
        result = subprocess.run(command, env=environment, stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        with (attempt / 'build.log').open('rb') as log:
            log.seek(max(0, log.seek(0, 2) - 4096))
            detail = '\n'.join(log.read().decode(errors='replace').strip().splitlines()[-16:])
        raise ValueError(f'Xwayland native build failed; see {attempt / "build.log"}\n{detail}')
    return attempt / 'xwayland.payload'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=[TARGET])
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    configured = os.environ.get('TELORGON_XWAYLAND_BUILD_ROOT')
    default_root = repository / 'target/xwayland-build/root'
    build_root = Path(configured).resolve(strict=True) if configured else (
        default_root.resolve() if default_root.is_dir() else None)
    cache = Path(os.environ.get('TELORGON_XWAYLAND_CACHE', repository / 'target/xwayland-cache')).resolve()
    print(ensure(repository, cache, args.target, build_root, build))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'Telorgon: {error}', file=sys.stderr)
        raise SystemExit(1)
