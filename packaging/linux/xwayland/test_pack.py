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
from abi_audit import AUDIT_PATH, REPOSITORY
from native_inputs import locked_sources, locked_target


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
        source = locked_sources(REPOSITORY)["xwayland"]
        self.components = [dict(name="xwayland", version=source["version"],
                                input_sha256=source["sha256"], license="synthetic test fixture")]
        self.audit = dict(schema=1, target=locked_target(REPOSITORY), minimum_glibc="2.2.5",
                          xwayland_version=source["version"], xwayland_source_sha256=source["sha256"],
                          files={relative: dict(needed=["libc.so.6"], glibc_floor="2.2.5",
                                 sha256=hashlib.sha256((self.stage / relative).read_bytes()).hexdigest())
                                 for relative in ("bin/Xwayland", "bin/xkbcomp")})
        self.write_audit()

    def write_audit(self):
        (self.stage / AUDIT_PATH).write_text(json.dumps(self.audit))

    def manifest(self):
        archive = pack.pack(self.stage, self.components)
        length, = struct.unpack("<I", archive[8:12])
        return json.loads(archive[12:12 + length])

    def test_deterministic_round_trip(self):
        archive = pack.pack(self.stage, self.components)
        self.assertEqual(archive, pack.pack(self.stage, self.components))
        self.assertEqual(archive[:8], b"TLXWP001")
        n, = struct.unpack("<I", archive[8:12])
        manifest = json.loads(archive[12:12+n])
        self.assertEqual(manifest["minimum_glibc"], "2.2.5")
        self.assertEqual(manifest["xwayland_version"], self.components[0]["version"])
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

    def test_private_library_can_raise_floor_beyond_old_build_baseline(self):
        library = self.stage / "lib/libXau.so.6"
        library.parent.mkdir()
        library.write_bytes((self.stage / "bin/Xwayland").read_bytes())
        self.audit["files"]["bin/Xwayland"]["needed"].append(library.name)
        self.audit["files"]["lib/libXau.so.6"] = dict(
            sha256=hashlib.sha256(library.read_bytes()).hexdigest(), needed=["libc.so.6"], glibc_floor="2.43")
        self.audit["minimum_glibc"] = "2.43"
        self.write_audit()
        self.assertEqual(self.manifest()["minimum_glibc"], "2.43")

    def test_rejects_stale_relocated_bytes(self):
        with (self.stage / "bin/Xwayland").open("ab") as helper:
            helper.write(b"relocation changed ELF bytes")
        with self.assertRaisesRegex(ValueError, "stale ELF audit hash"):
            pack.pack(self.stage, self.components)

    def test_rejects_missing_or_extra_audit_entries(self):
        for relative in ("bin/Xwayland", "lib/libXau.so.6"):
            with self.subTest(relative=relative):
                files = dict(self.audit["files"])
                if relative in files:
                    self.audit["files"].pop(relative)
                else:
                    self.audit["files"][relative] = files["bin/Xwayland"]
                self.write_audit()
                with self.assertRaisesRegex(ValueError, "cover every staged bin/lib file exactly"):
                    pack.pack(self.stage, self.components)
                self.audit["files"] = files

    def test_rejects_new_library_without_audit(self):
        library = self.stage / "lib/libXau.so.6"
        library.parent.mkdir()
        library.write_bytes((self.stage / "bin/Xwayland").read_bytes())
        with self.assertRaisesRegex(ValueError, "cover every staged bin/lib file exactly"):
            pack.pack(self.stage, self.components)

    def test_rejects_underreported_floor_and_missing_dependency(self):
        self.audit["files"]["bin/Xwayland"]["glibc_floor"] = "2.43"
        self.write_audit()
        with self.assertRaisesRegex(ValueError, "does not match its file requirements"):
            pack.pack(self.stage, self.components)
        self.audit["minimum_glibc"] = "2.43"
        self.audit["files"]["bin/Xwayland"]["needed"].append("libXau.so.6")
        self.write_audit()
        with self.assertRaisesRegex(ValueError, "missing or undeclared private dependency"):
            pack.pack(self.stage, self.components)

    def test_rejects_stale_source_provenance_and_obsolete_audit(self):
        for mutation in ("component", "audit", "obsolete", "missing"):
            with self.subTest(mutation=mutation):
                original = self.components[0]["version"], self.audit.copy()
                if mutation == "component":
                    self.components[0]["version"] = "999.0"
                elif mutation == "audit":
                    self.audit["xwayland_source_sha256"] = "0" * 64
                elif mutation == "obsolete":
                    self.audit = self.audit["files"]
                self.write_audit()
                if mutation == "missing":
                    (self.stage / AUDIT_PATH).unlink()
                with self.assertRaises(ValueError):
                    pack.pack(self.stage, self.components)
                self.components[0]["version"], self.audit = original

    def test_rejects_noncanonical_floor_and_duplicate_audit_fields(self):
        for floor in ("02.43", "2.043", "1.43", "2.65536", "2.43.0.1", None, 2):
            with self.subTest(floor=floor):
                self.audit["minimum_glibc"] = floor
                self.write_audit()
                with self.assertRaises(ValueError):
                    pack.pack(self.stage, self.components)
        (self.stage / AUDIT_PATH).write_text('{"schema":1,"schema":1}')
        with self.assertRaisesRegex(ValueError, "duplicate ELF audit field"):
            pack.pack(self.stage, self.components)


if __name__ == "__main__":
    unittest.main()
