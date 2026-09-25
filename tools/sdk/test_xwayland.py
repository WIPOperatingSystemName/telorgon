"""Native source-cache integrity and offline preparation checks."""
import hashlib
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

from source_cache import prepare

RECIPES = Path(__file__).resolve().parents[2] / 'third_party/recipes/xwayland'
sys.path.insert(0, str(RECIPES))
from dependencies import extract, load


class SourceCacheTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.cache = Path(self.directory.name)
        self.data = b'pinned archive bytes'
        self.source = dict(name='example', archive='example.tar.gz',
                           url='https://example.test/example.tar.gz',
                           sha256=hashlib.sha256(self.data).hexdigest())

    def test_verified_cache_is_reused_without_network(self):
        (self.cache / self.source['archive']).write_bytes(self.data)
        with patch('source_cache.urlopen', side_effect=AssertionError('network')):
            prepare(self.cache, [self.source], fetch=True)

    def test_bad_download_is_never_published(self):
        response = io.BytesIO(b'incorrect archive')
        response.url = self.source['url']
        with patch('source_cache.urlopen', return_value=response):
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                prepare(self.cache, [self.source], fetch=True)
        self.assertEqual(list(self.cache.iterdir()), [])

    def test_corrupted_cached_archive_fails_without_replacing_it(self):
        path = self.cache / self.source['archive']
        path.write_bytes(b'corrupted')
        with patch('source_cache.urlopen', side_effect=AssertionError('network')):
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                prepare(self.cache, [self.source], fetch=True)
        self.assertEqual(path.read_bytes(), b'corrupted')

    def test_missing_offline_input_has_an_actionable_error(self):
        with self.assertRaisesRegex(FileNotFoundError, '--fetch'):
            prepare(self.cache, [self.source])

    def test_archive_cannot_escape_cache(self):
        self.source['archive'] = '../escape.tar.gz'
        with self.assertRaisesRegex(ValueError, 'archive name'):
            prepare(self.cache, [self.source], fetch=True)

    def test_extraction_rejects_foreign_root_before_writing(self):
        path = self.cache / self.source['archive']
        with tarfile.open(path, 'w:gz') as archive:
            info = tarfile.TarInfo('unexpected/file')
            info.size = 1
            archive.addfile(info, io.BytesIO(b'x'))
        self.source.update(sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                           source_root='expected')
        output = self.cache / 'sources'
        output.mkdir()
        with self.assertRaisesRegex(ValueError, 'unexpected source root'):
            extract(self.cache, output, [self.source])
        self.assertEqual(list(output.iterdir()), [])

    def test_dependency_order_has_unique_names_and_required_libraries(self):
        entries = load(RECIPES)
        names = [entry['name'] for entry in entries]
        self.assertEqual(len(names), len(set(names)))
        for before, after in [('libffi', 'wayland'), ('expat', 'wayland'),
                              ('zlib', 'libpng'), ('libpng', 'freetype'),
                              ('libXau', 'libxcb'), ('xcb-proto', 'libxcb'),
                              ('libxcb', 'libX11'), ('libX11', 'libxkbfile')]:
            self.assertLess(names.index(before), names.index(after))


if __name__ == '__main__':
    unittest.main()
