"""Synthetic archive tests. No helper binary is built or executed."""
import hashlib
import json
from pathlib import Path
import struct
import tempfile
import tomllib
import unittest
import zlib
import pack


class PackTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="telorgon-pack-test-")
        self.addCleanup(self.temporary.cleanup)
        self.stage = Path(self.temporary.name) / "stage with spaces;$(literal)"
        self.stage.mkdir()
        elf = bytearray(64)
        elf[:6] = b"\x7fELF\x02\x01"
        elf[18:20] = b"\x3e\x00"
        for name, raw in {"bin/Xwayland": elf, "bin/xkbcomp": elf,
                          "share/X11/xkb/symbols/test": b"synthetic data",
                          "licenses/test.txt": b"synthetic fixture; never execute"}.items():
            path = self.stage / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(raw)
        self.components = [dict(name="test-only", version="0", input_sha256="0" * 64, license="synthetic test fixture")]

    def test_deterministic_round_trip(self):
        archive = pack.pack(self.stage, self.components)
        self.assertEqual(archive, pack.pack(self.stage, self.components))
        self.assertEqual(archive[:8], b"TLXWP001")
        n, = struct.unpack("<I", archive[8:12])
        manifest = json.loads(archive[12:12+n])
        data = archive[12+n:]
        offset = 0
        for entry in manifest["entries"]:
            self.assertEqual(entry["offset"], offset)
            compressed = data[offset:offset+entry["compressed_size"]]
            raw = zlib.decompress(compressed, -15)
            self.assertEqual(raw, (self.stage / entry["path"]).read_bytes())
            self.assertEqual(hashlib.sha256(raw).hexdigest(), entry["sha256"])
            self.assertEqual(len(raw), entry["size"])
            offset += len(compressed)
        self.assertEqual(offset, len(data))

    def test_rejects_file_and_directory_symlinks(self):
        for target in ["bin/Xwayland", "bin"]:
            with self.subTest(target=target):
                alias = self.stage / "alias"
                alias.symlink_to(target)
                with self.assertRaises(ValueError):
                    pack.pack(self.stage, self.components)
                alias.unlink()

    def test_rejects_stale_stages_that_shadow_host_graphics_dependencies(self):
        libraries = self.stage / "lib"
        libraries.mkdir()
        for name in ("libwayland-client.so.0", "libEGL.so.1", "libgbm.so.1"):
            with self.subTest(name=name):
                library = libraries / name
                library.write_bytes((self.stage / "bin/Xwayland").read_bytes())
                with self.assertRaisesRegex(ValueError, "shadows a host runtime dependency"):
                    pack.pack(self.stage, self.components)
                library.unlink()

    def test_wayland_client_shares_the_host_driver_runtime(self):
        policy = tomllib.loads(Path(pack.__file__).with_name("runtime-policy.toml").read_text())
        self.assertIn("libwayland-client.so.0", policy["host"])
        self.assertFalse(set(policy["host"]) & set(policy["private"]))

    def test_rejects_hardlinks(self):
        (self.stage / "hardlink").hardlink_to(self.stage / "bin/Xwayland")
        with self.assertRaises(ValueError):
            pack.pack(self.stage, self.components)

    def test_rejects_missing_helper_and_non_elf_helper(self):
        helper = self.stage / "bin/xkbcomp"
        helper.unlink()
        with self.assertRaises(ValueError):
            pack.pack(self.stage, self.components)
        helper.write_bytes(b"#!/bin/sh\nexit 0\n")
        with self.assertRaises(ValueError):
            pack.pack(self.stage, self.components)

    def test_rejects_compressed_and_expanded_overflow(self):
        for bound in ["MAX_COMPRESSED", "MAX_EXPANDED", "MAX_MANIFEST"]:
            old = getattr(pack, bound)
            try:
                setattr(pack, bound, 1)
                with self.assertRaises(ValueError):
                    pack.pack(self.stage, self.components)
            finally:
                setattr(pack, bound, old)


if __name__ == "__main__":
    unittest.main()
