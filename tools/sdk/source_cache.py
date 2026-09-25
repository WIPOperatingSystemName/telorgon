"""Fetch and verify pinned native source archives; never execute downloaded code."""
import hashlib
from pathlib import Path
import re
import tempfile
from urllib.parse import urlparse
from urllib.request import urlopen


def archive_name(source):
    name = source.get('archive', Path(urlparse(source['url']).path).name)
    if not name or Path(name).name != name or name in ('.', '..'):
        raise ValueError('invalid source archive name')
    return name


def verify(path, expected):
    if not re.fullmatch(r'[0-9a-f]{64}', expected):
        raise ValueError('invalid source SHA-256')
    with path.open('rb') as stream:
        if hashlib.file_digest(stream, 'sha256').hexdigest() != expected:
            raise ValueError(f'source digest mismatch: {path}')


def prepare(cache, sources, fetch=False):
    """A failed download never replaces a previously cached archive."""
    cache.mkdir(parents=True, exist_ok=True)
    names = [archive_name(source) for source in sources]
    if len(names) != len(set(names)):
        raise ValueError('duplicate source archive names')
    for source, name in zip(sources, names):
        path = cache / name
        if path.exists():
            verify(path, source['sha256'])
            continue
        if not fetch:
            raise FileNotFoundError(f'missing source: {path}; pass --fetch to download')
        if urlparse(source['url']).scheme != 'https':
            raise ValueError('source downloads require HTTPS')
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=cache, prefix='.download-', delete=False) as output:
                temporary = Path(output.name)
                with urlopen(source['url'], timeout=60) as response:
                    if urlparse(response.url).scheme != 'https':
                        raise ValueError('source download redirected away from HTTPS')
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
            verify(temporary, source['sha256'])
            temporary.replace(path)
            print(f'Downloaded and verified {name}', flush=True)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
