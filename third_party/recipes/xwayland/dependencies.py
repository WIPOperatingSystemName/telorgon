"""Compile pinned library sources into a private prefix, without system installation."""
import hashlib
from pathlib import Path
import shutil
import tarfile
import tomllib


def load(recipes):
    return tomllib.loads((recipes / 'dependencies.lock.toml').read_text())['source']


def extract(cache, sources, entries):
    for entry in entries:
        archive = cache / entry['archive']
        # Verify the exact open file that will be extracted, not a previous path lookup.
        with archive.open('rb') as data:
            if hashlib.file_digest(data, 'sha256').hexdigest() != entry['sha256']:
                raise ValueError(f'source digest mismatch: {archive}')
            data.seek(0)
            with tarfile.open(fileobj=data) as stream:
                if any(Path(member.name).parts[0] != entry['source_root']
                       for member in stream.getmembers()):
                    raise ValueError(f'unexpected source root: {archive}')
                stream.extractall(sources, filter='data')


def compile_libraries(entries, sources, build, private, run, jobs):
    for entry in entries:
        source = sources / entry['source_root']
        directory = build / entry['name']
        options = entry['options']
        kind = entry['build']
        if kind == 'meson':
            run('meson', 'setup', directory, source, '--prefix=' + str(private),
                '--libdir=lib', '--buildtype=release', '--default-library=shared',
                '--wrap-mode=nodownload', *options)
            run('meson', 'compile', '-C', directory, '-j', str(jobs))
            run('meson', 'install', '-C', directory, '--no-rebuild')
        elif kind == 'cmake':
            run('cmake', '-S', source, '-B', directory, '-DCMAKE_BUILD_TYPE=Release',
                '-DCMAKE_INSTALL_PREFIX=' + str(private), '-DCMAKE_INSTALL_LIBDIR=lib',
                '-DBUILD_SHARED_LIBS=ON', '-DBROTLI_DISABLE_TESTS=ON', *options)
            run('cmake', '--build', directory, '--parallel', str(jobs))
            run('cmake', '--install', directory)
        elif kind in ('autotools', 'zlib'):
            directory.mkdir()
            run(source / 'configure', '--prefix=' + str(private), *options, cwd=directory)
            run('make', '-j' + str(jobs), cwd=directory)
            run('make', 'install', cwd=directory)
        elif kind == 'bzip2':
            # Upstream's shared-library makefile has no install target.
            run('make', '-f', 'Makefile-libbz2_so', '-j' + str(jobs), cwd=source)
            (private / 'include').mkdir(parents=True, exist_ok=True)
            (private / 'lib').mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source / 'bzlib.h', private / 'include/bzlib.h')
            library = 'libbz2.so.' + entry['version']
            shutil.copyfile(source / library, private / 'lib' / library)
            for alias in ('libbz2.so', 'libbz2.so.1', 'libbz2.so.1.0'):
                (private / 'lib' / alias).symlink_to(library)
        else:
            raise ValueError(f'unknown native build system: {kind}')
