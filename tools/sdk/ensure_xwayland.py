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
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile
import tomllib

from source_cache import prepare
from native_inputs import TARGET, glibc_version, host_glibc, locked_sources

INPUTS = (
    'tools/sdk/ensure_xwayland.py', 'tools/sdk/build_xwayland.py',
    'tools/sdk/source_cache.py', 'tools/sdk/native_inputs.py', 'third_party/sources.lock.toml',
    'third_party/recipes/xwayland', 'third_party/patches/xwayland',
    'packaging/linux/xwayland/stage.py', 'packaging/linux/xwayland/pack.py',
    'packaging/linux/xwayland/abi_audit.py',
    'packaging/linux/xwayland/runtime-policy.toml',
    'crates/telorgon/src/integrations/x11/payload_format.rs',
)
HOST_TOOLS = ('cc', 'c++', 'ld', 'ar', 'ranlib', 'meson', 'ninja', 'make', 'patch',
              'pkg-config', 'readelf', 'patchelf', 'cmake', 'perl', 'bison', 'byacc')
HOST_PACKAGES = ('libdrm', 'gbm', 'egl', 'gl', 'dri')
SANITIZED_VARIABLES = ('CC', 'CXX', 'AR', 'CFLAGS', 'CXXFLAGS', 'CPPFLAGS', 'LDFLAGS',
                       'LD_LIBRARY_PATH', 'LD_PRELOAD', 'LIBRARY_PATH', 'LD_RUN_PATH',
                       'CPATH', 'C_INCLUDE_PATH', 'CPLUS_INCLUDE_PATH', 'OBJC_INCLUDE_PATH',
                       'PKG_CONFIG_PATH', 'PKG_CONFIG_LIBDIR', 'PKG_CONFIG_SYSROOT_DIR',
                       'CMAKE_PREFIX_PATH', 'PYTHONPATH', 'DESTDIR', 'CONFIG_SITE')


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


def sanitized_environment():
    environment = dict(os.environ)
    # Native identity discovery and compilation must see the same search paths.
    for name in SANITIZED_VARIABLES:
        environment.pop(name, None)
    environment['PYTHONDONTWRITEBYTECODE'] = '1'
    return environment


def probe(command, limit=8192, allow_failure=False):
    """Keep host identity discovery bounded and independent of build success."""
    try:
        result = subprocess.run(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                env=sanitized_environment(), check=False, timeout=10)
        if (result.returncode and not allow_failure) or len(result.stdout) > limit:
            return None
        return result.stdout.decode(errors='replace').strip()
    except (OSError, subprocess.TimeoutExpired):
        return None


def file_identity(path):
    path = Path(path)
    if not path.is_file():
        return None
    resolved = path.resolve()
    return {'path': str(resolved), 'input': str(path.absolute()), 'sha256': digest(resolved)}


def relevant_packages(paths):
    """Identify package revisions owning build tools, libraries and their headers."""
    paths = sorted(set(paths))
    dpkg = shutil.which('dpkg-query')
    pacman = shutil.which('pacman')
    rpm = shutil.which('rpm')
    if dpkg:
        owners = probe([dpkg, '-S', *paths], limit=128 * 1024, allow_failure=True)
        packages = set()
        for line in (owners or '').splitlines():
            if ': ' not in line:
                continue
            for name in line.rsplit(': ', 1)[0].split(', '):
                if re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9+.-]*(?::[A-Za-z0-9_-]+)?', name):
                    packages.add(name)
        inventory = (probe([dpkg, '-W', '-f=${binary:Package}\t${Version}\t${Architecture}\n',
                            *sorted(packages)], limit=128 * 1024) if packages else None)
        databases = [Path('/var/lib/dpkg/status')]
    elif pacman:
        owners = probe([pacman, '-Qoq', *paths], limit=128 * 1024, allow_failure=True)
        packages = sorted({name for name in (owners or '').splitlines()
                           if re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9+._-]*', name)})
        inventory = probe([pacman, '-Q', *packages], limit=128 * 1024) if packages else None
        databases = [Path('/var/lib/pacman/local')]
    elif rpm:
        inventory = probe([rpm, '-qf', '--qf', '%{NAME}\t%{VERSION}-%{RELEASE}\t%{ARCH}\n',
                           *paths], limit=128 * 1024, allow_failure=True)
        databases = [Path('/var/lib/rpm'), Path('/usr/lib/sysimage/rpm')]
    else:
        return None, []
    return sorted((inventory or '').splitlines()), [str(path) for path in databases if path.exists()]


def watched_files(value):
    if isinstance(value, dict):
        for name, child in value.items():
            if name in ('path', 'input') and isinstance(child, str):
                yield child
            else:
                yield from watched_files(child)


def host_inputs():
    identity = {'libc': platform.libc_ver(), 'machine': platform.machine()}
    tools = {}
    for name in HOST_TOOLS:
        path = shutil.which(name)
        tools[name] = ({'file': file_identity(path), 'version': probe([path, '--version'])}
                       if path else None)
    tools['python'] = file_identity(sys.executable)
    identity['tools'] = tools
    compiler = shutil.which('cc')
    if compiler:
        # Driver hashes alone do not identify GCC's separately installed front ends
        # or distribution patches to libc with the same ABI version number.
        identity['compiler_inputs'] = {
            name: file_identity(path) if path else None
            for name in ('cc1', 'cc1plus', 'collect2', 'libc.so.6', 'libm.so.6', 'libstdc++.so')
            for path in [probe([compiler, ('-print-prog-name=' if name in ('cc1', 'cc1plus', 'collect2')
                                          else '-print-file-name=') + name])]
        }
    pkg_config = shutil.which('pkg-config')
    if pkg_config:
        interfaces = {}
        for name in HOST_PACKAGES:
            record = {option: probe([pkg_config, option, name])
                      for option in ('--modversion', '--cflags', '--libs')}
            directory = probe([pkg_config, '--variable=pcfiledir', name])
            record['pc'] = file_identity(Path(directory) / (name + '.pc')) if directory else None
            if compiler:
                record['libraries'] = {
                    option[2:]: file_identity(path) if path else None
                    for option in shlex.split(record['--libs'] or '') if option.startswith('-l')
                    for path in [probe([compiler, '-print-file-name=lib' + option[2:] + '.so'])]
                }
            interfaces[name] = record
        identity['native_interfaces'] = interfaces
    # Package revisions also identify patched headers and build-tool modules that
    # are not represented by the executables or pkg-config ABI version strings.
    identity['headers'] = {name: file_identity(Path('/usr/include') / name)
                           for name in ('stdio.h', 'features.h', 'pthread.h', 'gbm.h',
                                        'libdrm/drm.h', 'EGL/egl.h', 'GL/gl.h')}
    paths = list(watched_files(identity))
    identity['packages'], databases = relevant_packages(paths)
    identity['watch_paths'] = sorted(set(paths + databases))
    return identity


def native_environment(build_root):
    environment = {'root': str(build_root) if build_root else 'host'}
    watch_paths = []
    if build_root:
        # Changes to the prepared native SDK invalidate binaries built against it.
        for relative in ('var/lib/dpkg/status', 'etc/os-release', 'usr/local/bin/cmake'):
            path = build_root / relative
            if path.exists():
                environment[relative] = digest(path)
                watch_paths.append(str(path))
    else:
        environment['host'] = host_inputs()
        watch_paths.extend(environment['host']['watch_paths'])
    environment['watch_paths'] = watch_paths
    return environment


def fingerprint(repository, target, build_root, environment=None):
    records = [(str(path.relative_to(repository)), digest(path))
               for path in input_files(repository)]
    environment = environment if environment is not None else native_environment(build_root)
    environment = dict(environment, target=target)
    return hashlib.sha256(json.dumps([records, environment], sort_keys=True).encode()).hexdigest()


def payload_header(path, target, source):
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
    if (manifest.get('schema'), manifest.get('target'), manifest.get('xwayland_version')) != (
            1, target, source['version']):
        raise ValueError('incompatible Xwayland payload')
    glibc_version(manifest.get('minimum_glibc'))
    # Cargo performs the full archive validation before embedding these bytes.


def cached_payload(cache, key, target, source):
    payload = cache / 'xwayland.payload'
    try:
        marker = json.loads((cache / 'build.json').read_text())
        if marker['fingerprint'] != key or marker['sha256'] != digest(payload):
            return None
        payload_header(payload, target, source)
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
        source = locked_sources(repository)['xwayland']
        environment = native_environment(build_root)
        key = fingerprint(repository, target, build_root, environment)
        hit = cached_payload(cache, key, target, source)
        if hit:
            return hit
        attempt = Path(tempfile.mkdtemp(prefix='build-', dir=cache))
        print(f'Telorgon: preparing Xwayland; build log: {attempt / "build.log"}', file=sys.stderr, flush=True)
        candidate = builder(repository, attempt, target, build_root)
        payload_header(candidate, target, source)
        if fingerprint(repository, target, build_root) != key:
            raise ValueError('Xwayland build inputs changed during compilation; retry the build')
        # Preserve the old working payload on build failure. The marker is published last;
        # an interruption between replacements becomes a cache miss on the next invocation.
        payload_hash = digest(candidate)
        candidate.replace(cache / 'xwayland.payload')
        atomic_json(cache / 'build.json', {'fingerprint': key, 'sha256': payload_hash,
                                         'inputs': environment['watch_paths']})
        return cache / 'xwayland.payload'


def build(repository, attempt, target, build_root):
    if not build_root:
        host_glibc()
        from build_xwayland import preflight
        failures = preflight(with_dependencies=True, environment=sanitized_environment())
        if failures:
            raise ValueError('Xwayland build prerequisites are missing:\n' + '\n'.join(failures))
    sources = list(locked_sources(repository).values())
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
    environment = sanitized_environment()
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
        command = [sys.executable, str(repository / 'tools/sdk/build_xwayland.py'), *arguments]
    with (attempt / 'build.log').open('w') as log:
        result = subprocess.run(command, env=environment, stdout=log, stderr=subprocess.STDOUT)
    if result.returncode:
        with (attempt / 'build.log').open('rb') as log:
            log.seek(max(0, log.seek(0, 2) - 4096))
            detail = '\n'.join(log.read().decode(errors='replace').strip().splitlines()[-16:])
        raise ValueError(f'Xwayland native build failed; see {attempt / "build.log"}\n{detail}')
    return attempt / 'xwayland.payload'


def configured_build_root():
    configured = os.environ.get('TELORGON_XWAYLAND_BUILD_ROOT')
    if configured:
        root = Path(configured).resolve(strict=True)
        if not root.is_dir():
            raise ValueError('TELORGON_XWAYLAND_BUILD_ROOT must name a prepared directory')
        return root
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', required=True, choices=[TARGET])
    args = parser.parse_args()
    repository = Path(__file__).resolve().parents[2]
    build_root = configured_build_root()
    cache = Path(os.environ.get('TELORGON_XWAYLAND_CACHE', repository / 'target/xwayland-cache')).resolve()
    print(ensure(repository, cache, args.target, build_root, build))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'Telorgon: {error}', file=sys.stderr)
        raise SystemExit(1)
