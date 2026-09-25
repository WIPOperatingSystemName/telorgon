"""Offline PipeWire client SDK recipe; never starts or replaces the host server.

Outputs a private development/runtime prefix and retains matching native sources.
Build in the existing pinned x86-64 Ubuntu root. Rust/bindgen and graphical dependencies
remain the caller's toolchain; this recipe owns PipeWire/SPA only.
"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tomllib

PREFIX = Path('/opt/telorgon/pipewire')


def load_lock(recipe):
    return tomllib.loads((recipe / 'source.lock.toml').read_text())


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verify_archive(archive, lock):
    if digest(archive) != lock['sha256']:
        raise ValueError(f'PipeWire source digest mismatch: {archive}')


def verify_environment(recipe, lock):
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise ValueError('requires the pinned x86-64 Linux build root')
    libc = ctypes.CDLL(None)
    libc.gnu_get_libc_version.restype = ctypes.c_char_p
    if libc.gnu_get_libc_version().decode() != lock['minimum_glibc']:
        raise ValueError('requires glibc ' + lock['minimum_glibc'])
    inventory = subprocess.check_output(
        ['/usr/bin/dpkg-query', '-W', '-f=${binary:Package}\t${Version}\t${Architecture}\n'])
    if inventory != (recipe / lock['build_inventory']).read_bytes():
        raise ValueError('build package inventory differs from the pinned native build root')
    return inventory


def compile_sdk(work, archive, recipe, jobs):
    lock = load_lock(recipe)
    verify_archive(archive, lock)
    inventory = verify_environment(recipe, lock)
    work.mkdir(parents=True, exist_ok=False)
    source_dir = work / 'sources'
    source_dir.mkdir()
    # Copy and verify once more before extracting so the caller's cache can be shared.
    locked_archive = work / lock['archive']
    shutil.copyfile(archive, locked_archive)
    verify_archive(locked_archive, lock)
    with tarfile.open(locked_archive) as source:
        for member in source.getmembers():
            if Path(member.name).parts[0] != lock['source_root']:
                raise ValueError('archive has an unexpected source root')
        source.extractall(source_dir, filter='data')
    source = source_dir / lock['source_root']
    build = work / 'build'
    stage = work / 'stage'
    private = stage / PREFIX.relative_to('/')
    # No inherited compiler/linker flags, pkg-config paths, user caches or install roots.
    env = {
        'PATH': '/usr/bin:/bin', 'LC_ALL': 'C', 'TZ': 'UTC',
        'SOURCE_DATE_EPOCH': '0', 'CC': '/usr/bin/gcc', 'CXX': '/usr/bin/g++',
        'PKG_CONFIG': '/usr/bin/pkg-config',
        'CFLAGS': f'-O2 -march=x86-64 -mtune=generic -ffile-prefix-map={work}=/src/telorgon-native',
        'CXXFLAGS': f'-O2 -march=x86-64 -mtune=generic -ffile-prefix-map={work}=/src/telorgon-native',
        'DESTDIR': str(stage),
    }
    commands = []

    def run(*args):
        argv = list(map(str, args))
        commands.append(argv)
        (work / 'build-commands.json').write_text(json.dumps(commands, indent=2) + '\n')
        subprocess.run(argv, cwd=work, env=env, check=True)

    run('/usr/bin/meson', 'setup', build, source, '--prefix=' + str(PREFIX),
        '--libdir=lib', '--sysconfdir=etc', '--buildtype=release',
        '--wrap-mode=nodownload', '-Dauto_features=disabled', '-Ddefault_library=shared',
        '-Dsession-managers=[]', '-Dtests=disabled', '-Dinstalled_tests=disabled',
        '-Dexamples=disabled', '-Ddocs=disabled', '-Dman=disabled',
        '-Dpipewire-jack=disabled', '-Dpipewire-v4l2=disabled', '-Ddbus=disabled',
        '-Dflatpak=disabled', '-Dlegacy-rtkit=false', '-Drlimits-install=false',
        '-Dpam-defaults-install=false', '-Dspa-plugins=enabled', '-Dsupport=enabled',
        '-Daudioconvert=enabled', '-Daudiomixer=enabled', '-Dvideoconvert=enabled',
        '-Dcontrol=enabled', '-Daudiotestsrc=disabled', '-Dvideotestsrc=disabled')
    run('/usr/bin/meson', 'compile', '-C', build, '-j', str(jobs))
    run('/usr/bin/meson', 'install', '-C', build, '--no-rebuild')
    # Daemons/tools built unconditionally upstream are not part of the client SDK.
    # Only remove files inside the newly created private staging prefix.
    for relative in ('bin', 'etc', 'share/man', 'share/locale', 'share/bash-completion', 'share/zsh'):
        path = private / relative
        if path.is_dir():
            shutil.rmtree(path)
    config_dir = private / 'share/pipewire'
    for path in config_dir.iterdir():
        if path.name != 'client.conf':
            if path.is_dir():
                shutil.rmtree(path)
            else:
                path.unlink()
    # Installed .pc files must locate staged headers/libraries without a system sysroot.
    for pc in (private / 'lib/pkgconfig').glob('*.pc'):
        pc.write_text(pc.read_text().replace('prefix=' + str(PREFIX), 'prefix=' + str(private)))
    provenance = private / 'share/telorgon-native'
    provenance.mkdir(parents=True)
    shutil.copyfile(locked_archive, provenance / lock['archive'])
    shutil.copyfile(recipe / 'source.lock.toml', provenance / 'source.lock.toml')
    shutil.copyfile(recipe / 'recipe.py', provenance / 'recipe.py')
    shutil.copyfile(source / 'COPYING', provenance / 'COPYING')
    (provenance / 'build-packages.tsv').write_bytes(inventory)
    shutil.copyfile(recipe / lock['build_archives'], provenance / 'build-debs.lock.json')
    # Manifest is written last: a partial build can never masquerade as usable.
    files = {}
    for path in sorted(private.rglob('*')):
        relative = str(path.relative_to(private))
        if path.is_symlink():
            if not path.resolve().is_relative_to(private):
                raise ValueError('installed symlink escapes private prefix: ' + relative)
            files[relative] = {'symlink': os.readlink(path)}
        elif path.is_file():
            files[relative] = {'sha256': digest(path)}
    manifest = {'schema': 1, 'source_sha256': lock['sha256'], 'files': files}
    (private / 'telorgon-native.json').write_text(json.dumps(manifest, indent=2) + '\n')
    return private


def runtime_environment(private, recipe):
    lock = load_lock(recipe)
    manifest = json.loads((private / 'telorgon-native.json').read_text())
    if manifest['source_sha256'] != lock['sha256']:
        raise ValueError('private prefix uses a different locked source')
    for name, expected in manifest['files'].items():
        path = private / name
        if not path.resolve().is_relative_to(private):
            raise ValueError('manifest entry escapes private prefix')
        if 'symlink' in expected:
            if not path.is_symlink() or os.readlink(path) != expected['symlink']:
                raise ValueError('changed native symlink: ' + name)
        elif path.is_symlink() or digest(path) != expected['sha256']:
            raise ValueError('changed native artifact: ' + name)
    env = dict(os.environ)
    for key, directory in (
        ('PKG_CONFIG_PATH', private / 'lib/pkgconfig'),
        ('LD_LIBRARY_PATH', private / 'lib'),
    ):
        env[key] = str(directory) + (':' + env[key] if env.get(key) else '')
    # Native search overrides are scoped to the launched command, never the user session.
    env['PIPEWIRE_MODULE_DIR'] = str(private / 'lib/pipewire-0.3')
    env['SPA_PLUGIN_DIR'] = str(private / 'lib/spa-0.2')
    env['PIPEWIRE_CONFIG_DIR'] = str(private / 'share/pipewire')
    env['PIPEWIRE_CONFIG_NAME'] = 'client.conf'
    return env
