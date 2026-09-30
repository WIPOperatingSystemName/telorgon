"""Validated source identities and GNU platform inputs for native SDK builds."""
import ctypes
from pathlib import Path
import platform
import re
import tomllib

TARGET = 'x86_64-unknown-linux-gnu'
_COMPONENT = re.compile(r'[A-Za-z0-9][A-Za-z0-9._+-]{0,127}\Z')
_GLIBC = re.compile(r'(?:0|[1-9][0-9]{0,4})(?:\.(?:0|[1-9][0-9]{0,4})){1,2}\Z')


def glibc_version(value):
    """Parse bounded canonical ABI metadata, padding the optional patch component."""
    if not isinstance(value, str) or not _GLIBC.fullmatch(value):
        raise ValueError('invalid glibc requirement')
    parts = tuple(map(int, value.split('.')))
    if parts[0] < 2 or any(part > 65535 for part in parts):
        raise ValueError('invalid glibc requirement')
    return parts + (0,) * (3 - len(parts))


def host_glibc():
    if platform.system() != 'Linux' or platform.machine() != 'x86_64':
        raise ValueError('Xwayland builds currently require x86-64 Linux with glibc')
    try:
        version = ctypes.CDLL(None).gnu_get_libc_version
    except AttributeError as error:
        raise ValueError('Xwayland builds currently require GNU libc') from error
    version.restype = ctypes.c_char_p
    result = version().decode('ascii')
    glibc_version(result)
    return result


def source_directory(source):
    name, version = source.get('name'), source.get('version')
    if not all(isinstance(value, str) and _COMPONENT.fullmatch(value)
               for value in (name, version)):
        raise ValueError('invalid native source name or version')
    directory = source.get('source_root', f'{name}-{version}')
    if not isinstance(directory, str) or not _COMPONENT.fullmatch(directory):
        raise ValueError('invalid native source directory')
    return directory


def source_lock(repository):
    lock = tomllib.loads((Path(repository) / 'third_party/sources.lock.toml').read_text())
    if lock.get('schema') != 1 or lock.get('target') != TARGET:
        raise ValueError('unsupported native source lock schema or target')
    return lock


def locked_target(repository):
    return source_lock(repository)['target']


def locked_sources(repository):
    sources = {}
    entries = source_lock(repository).get('source')
    if not isinstance(entries, list):
        raise ValueError('missing native source entries')
    for source in entries:
        source_directory(source)
        name = source['name']
        if name in sources:
            raise ValueError(f'duplicate native source: {name}')
        if not isinstance(source.get('sha256'), str) or not re.fullmatch(r'[0-9a-f]{64}', source['sha256']):
            raise ValueError(f'invalid native source digest: {name}')
        sources[name] = source
    return sources
