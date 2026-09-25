"""Observable cache behavior for automatic Xwayland dependency preparation."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import struct
import tempfile
import threading
import unittest
from unittest.mock import patch

import ensure_xwayland as sdk


def payload_bytes(label):
    manifest = json.dumps({'schema': 1, 'target': sdk.TARGET, 'minimum_glibc': '2.39'}).encode()
    return b'TLXWP001' + struct.pack('<I', len(manifest)) + manifest + label.encode()


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.repository = Path(self.directory.name)
        (self.repository / 'sources.lock').write_text('v1')
        (self.repository / 'recipe.py').write_text('recipe v1')
        self.cache = self.repository / 'cache'
        self.patch = patch.object(sdk, 'INPUTS', ('sources.lock', 'recipe.py'))
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.builds = 0

    def builder(self, repository, attempt, target, root):
        self.builds += 1
        path = attempt / 'new.payload'
        path.write_bytes(payload_bytes(str(self.builds)))
        return path

    def ensure(self, builder=None):
        return sdk.ensure(self.repository, self.cache, sdk.TARGET, None, builder or self.builder)

    def test_unchanged_inputs_reuse_payload_without_building(self):
        first = self.ensure()
        second = self.ensure()
        self.assertEqual(first, second)
        self.assertEqual(self.builds, 1)

    def test_source_and_recipe_changes_each_trigger_one_build(self):
        self.ensure()
        for file in ('sources.lock', 'recipe.py'):
            (self.repository / file).write_text('changed')
            self.ensure()
            self.ensure()
        self.assertEqual(self.builds, 3)

    def test_corrupted_or_missing_payload_is_rebuilt(self):
        path = self.ensure()
        path.write_bytes(b'corrupted')
        self.ensure()
        path.unlink()
        self.ensure()
        self.assertEqual(self.builds, 3)

    def test_failed_rebuild_preserves_old_payload_and_marker(self):
        path = self.ensure()
        old = path.read_bytes()
        marker = (self.cache / 'build.json').read_bytes()
        (self.repository / 'sources.lock').write_text('v2')
        def fail(*args):
            raise ValueError('compiler failed')
        with self.assertRaisesRegex(ValueError, 'compiler failed'):
            self.ensure(fail)
        self.assertEqual(path.read_bytes(), old)
        self.assertEqual((self.cache / 'build.json').read_bytes(), marker)
        self.ensure()
        self.assertEqual(self.builds, 2)

    def test_changed_inputs_during_build_are_not_published(self):
        def change(*args):
            path = self.builder(*args)
            (self.repository / 'recipe.py').write_text('changed while building')
            return path
        with self.assertRaisesRegex(ValueError, 'inputs changed'):
            self.ensure(change)
        self.assertFalse((self.cache / 'build.json').exists())

    def test_invalid_builder_output_is_not_published(self):
        def invalid(*args):
            path = self.builder(*args)
            path.write_bytes(b'not an archive')
            return path
        with self.assertRaises(ValueError):
            self.ensure(invalid)
        self.assertFalse((self.cache / 'xwayland.payload').exists())

    def test_parallel_consumers_share_one_native_build(self):
        entered, release = threading.Event(), threading.Event()
        def build(*args):
            entered.set()
            if not release.wait(5):
                raise RuntimeError('test timed out')
            return self.builder(*args)
        with ThreadPoolExecutor(max_workers=2) as pool:
            first = pool.submit(self.ensure, build)
            self.assertTrue(entered.wait(5))
            second = pool.submit(self.ensure, build)
            release.set()
            self.assertEqual(first.result(timeout=5), second.result(timeout=5))
        self.assertEqual(self.builds, 1)

    def test_prepared_root_package_changes_invalidate_key(self):
        root = self.repository / 'root'
        status = root / 'var/lib/dpkg/status'
        status.parent.mkdir(parents=True)
        status.write_text('toolchain v1')
        first = sdk.fingerprint(self.repository, sdk.TARGET, root)
        status.write_text('toolchain v2')
        self.assertNotEqual(first, sdk.fingerprint(self.repository, sdk.TARGET, root))


if __name__ == '__main__':
    unittest.main()
