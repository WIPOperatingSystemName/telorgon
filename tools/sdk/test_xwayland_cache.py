"""Observable cache behavior for automatic Xwayland dependency preparation."""
from concurrent.futures import ThreadPoolExecutor
import contextlib
import io
import json
import os
from pathlib import Path
import struct
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

import ensure_xwayland as sdk


def payload_bytes(label, version='24.1.13', minimum_glibc='2.43'):
    manifest = json.dumps({'schema': 1, 'target': sdk.TARGET, 'minimum_glibc': minimum_glibc,
                           'xwayland_version': version}).encode()
    return b'TLXWP001' + struct.pack('<I', len(manifest)) + manifest + label.encode()


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.repository = Path(self.directory.name)
        self.source_lock = self.repository / 'third_party/sources.lock.toml'
        self.source_lock.parent.mkdir()
        self.write_pin('24.1.13')
        (self.repository / 'recipe.py').write_text('recipe v1')
        self.cache = self.repository / 'cache'
        self.patch = patch.object(sdk, 'INPUTS', ('third_party/sources.lock.toml', 'recipe.py'))
        self.patch.start()
        self.addCleanup(self.patch.stop)
        self.host = {'compiler': 'v1', 'library': 'v1', 'watch_paths': ['/native/compiler']}
        self.host_patch = patch.object(sdk, 'host_inputs', side_effect=lambda: dict(self.host))
        self.host_patch.start()
        self.addCleanup(self.host_patch.stop)
        self.builds = 0

    def write_pin(self, version):
        self.source_lock.write_text(
            f'schema = 1\ntarget = "{sdk.TARGET}"\n'
            f'[[source]]\nname = "xwayland"\nversion = "{version}"\n'
            'url = "https://example.test/xwayland.tar.xz"\nsha256 = "' + 'a' * 64 + '"\n')

    def builder(self, repository, attempt, target, root):
        self.builds += 1
        path = attempt / 'new.payload'
        version = sdk.locked_sources(repository)['xwayland']['version']
        path.write_bytes(payload_bytes(str(self.builds), version=version))
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
        for file in ('third_party/sources.lock.toml', 'recipe.py'):
            with (self.repository / file).open('a') as stream:
                stream.write('\n# changed\n')
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
        self.write_pin('24.1.14')
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

    def test_host_toolchain_and_native_library_updates_each_rebuild(self):
        self.ensure()
        for field in ('compiler', 'library'):
            self.host[field] = 'security update'
            self.ensure()
            self.ensure()
        self.assertEqual(self.builds, 3)
        marker = json.loads((self.cache / 'build.json').read_text())
        self.assertEqual(marker['inputs'], ['/native/compiler'])

    def test_source_pin_updates_version_and_invalidate_cache(self):
        self.ensure()
        self.write_pin('24.1.14')
        path = self.ensure()
        sdk.payload_header(path, sdk.TARGET, sdk.locked_sources(self.repository)['xwayland'])
        self.assertEqual(self.builds, 2)

    def test_old_version_payload_is_rejected_after_pin_update(self):
        self.write_pin('24.1.14')
        def old(*args):
            path = args[1] / 'old.payload'
            path.write_bytes(payload_bytes('old'))
            return path
        with self.assertRaisesRegex(ValueError, 'incompatible'):
            self.ensure(old)
        self.assertFalse((self.cache / 'build.json').exists())

    def test_payload_accepts_measured_floor_and_rejects_malformed_claims(self):
        path = self.cache / 'header.payload'
        path.parent.mkdir()
        source = sdk.locked_sources(self.repository)['xwayland']
        for floor in ('2.17', '2.43', '2.2.5'):
            path.write_bytes(payload_bytes('test', minimum_glibc=floor))
            sdk.payload_header(path, sdk.TARGET, source)
        for floor in ('2.043', '2.43junk', '2.43.0.1', '2.65536', '1.2', None):
            path.write_bytes(payload_bytes('test', minimum_glibc=floor))
            with self.assertRaisesRegex(ValueError, 'glibc requirement'):
                sdk.payload_header(path, sdk.TARGET, source)

    def test_default_build_ignores_previously_prepared_root(self):
        (self.repository / 'target/xwayland-build/root').mkdir(parents=True)
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(sdk, '__file__', str(self.repository / 'tools/sdk/ensure_xwayland.py')), \
                patch.object(sdk.sys, 'argv', ['ensure', '--target', sdk.TARGET]), \
                patch.object(sdk, 'ensure', return_value=self.cache / 'xwayland.payload') as ensure, \
                contextlib.redirect_stdout(io.StringIO()):
            sdk.main()
        self.assertIsNone(ensure.call_args.args[3])

    def test_explicit_prepared_root_is_supported(self):
        root = self.repository / 'prepared'
        root.mkdir()
        with patch.dict(os.environ, {'TELORGON_XWAYLAND_BUILD_ROOT': str(root)}):
            self.assertEqual(sdk.configured_build_root(), root.resolve())

    def test_preflight_fails_before_any_download(self):
        with patch.object(sdk, 'host_glibc', return_value='2.43'), \
                patch('build_xwayland.preflight', return_value=['missing build tool: meson']), \
                patch.object(sdk, 'prepare', side_effect=AssertionError('download')):
            with self.assertRaisesRegex(ValueError, 'missing build tool: meson'):
                sdk.build(self.repository, self.repository, sdk.TARGET, None)

    def test_host_glibc_243_uses_normal_native_build(self):
        dependencies = self.repository / 'third_party/recipes/xwayland/dependencies.lock.toml'
        dependencies.parent.mkdir(parents=True)
        dependencies.write_text('source = []\n')
        attempt = self.repository / 'attempt'
        attempt.mkdir()
        with patch.object(sdk, 'host_glibc', return_value='2.43'), \
                patch('build_xwayland.preflight', return_value=[]), \
                patch.object(sdk, 'prepare'), \
                patch.object(sdk.subprocess, 'run', return_value=Mock(returncode=0)) as run:
            output = sdk.build(self.repository, attempt, sdk.TARGET, None)
        self.assertEqual(output, attempt / 'xwayland.payload')
        self.assertEqual(run.call_args.args[0][0], sdk.sys.executable)
        self.assertIn('--with-dependencies', run.call_args.args[0])


class HostIdentityTests(unittest.TestCase):
    def test_compiler_frontend_patch_invalidates_identity_without_version_change(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            compiler, frontend = root / 'cc', root / 'cc1'
            compiler.write_bytes(b'compiler driver')
            frontend.write_bytes(b'frontend v1')
            def probe(command):
                if command[1] == '--version':
                    return 'compiler ABI version unchanged'
                if command[1] == '-print-prog-name=cc1':
                    return str(frontend)
                return None
            with patch.object(sdk, 'HOST_TOOLS', ('cc',)), \
                    patch.object(sdk.shutil, 'which', side_effect=lambda name: str(compiler) if name == 'cc' else None), \
                    patch.object(sdk, 'probe', side_effect=probe), \
                    patch.object(sdk, 'relevant_packages', return_value=([], [])):
                first = sdk.host_inputs()
                frontend.write_bytes(b'security-patched frontend')
                second = sdk.host_inputs()
            self.assertNotEqual(first, second)
            self.assertIn(str(frontend), second['watch_paths'])

    def test_package_identity_queries_only_owners_of_relevant_files(self):
        commands = []
        def probe(command, **kwargs):
            commands.append(command)
            if command[1] == '-S':
                return 'gcc-15: /usr/bin/cc\nlibc6-dev:amd64: /usr/include/stdio.h\n'
            return 'gcc-15\t15.2-1\tamd64\nlibc6-dev:amd64\t2.43-2\tamd64\n'
        with patch.object(sdk.shutil, 'which', side_effect=lambda name: '/usr/bin/dpkg-query' if name == 'dpkg-query' else None), \
                patch.object(sdk, 'probe', side_effect=probe):
            packages, _ = sdk.relevant_packages(['/usr/bin/cc', '/usr/include/stdio.h'])
        self.assertEqual(len(packages), 2)
        self.assertEqual(commands[0][2:], ['/usr/bin/cc', '/usr/include/stdio.h'])
        self.assertEqual(commands[1][-2:], ['gcc-15', 'libc6-dev:amd64'])

    def test_discovery_and_compile_clear_native_search_overrides(self):
        with patch.dict(os.environ, {'PATH': '/native/bin', 'CPATH': '/foreign/include',
                                     'PKG_CONFIG_PATH': '/foreign/pkgconfig', 'CFLAGS': '-march=native'}):
            environment = sdk.sanitized_environment()
        self.assertEqual(environment['PATH'], '/native/bin')
        self.assertNotIn('CPATH', environment)
        self.assertNotIn('PKG_CONFIG_PATH', environment)
        self.assertNotIn('CFLAGS', environment)


if __name__ == '__main__':
    unittest.main()
