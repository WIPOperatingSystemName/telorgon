"""Synthetic readelf output verifies ABI detection without executing helpers."""
import hashlib
from pathlib import Path
import tempfile
import tomllib
import unittest
from abi_audit import REPOSITORY, create_audit, inspect_elf
from native_inputs import source_lock


class AbiAuditTests(unittest.TestCase):
    @staticmethod
    def inspect(tags):
        def output(*args):
            return "\n".join(f"Name: GLIBC_{tag} Flags: none" for tag in tags) if "--version-info" in args else " (NEEDED) Shared library: [libc.so.6]"
        return inspect_elf(Path("synthetic-elf"), output)

    def test_numeric_requirements_compare_patch_components(self):
        self.assertEqual(self.inspect(["2.2.5", "2.43", "2.43.1"])["glibc_floor"], "2.43.1")
        self.assertEqual(self.inspect(["2.43", "2.43.0"])["glibc_floor"], "2.43")

    def test_relr_requirement_sets_loader_floor(self):
        self.assertEqual(self.inspect(["2.2.5", "ABI_DT_RELR"])["glibc_floor"], "2.36")

    def test_relr_dynamic_entries_require_new_loader_without_version_node(self):
        for tag in ("RELR", "RELRSZ", "RELRENT"):
            with self.subTest(tag=tag):
                def output(*args):
                    return "Name: GLIBC_2.2.5" if "--version-info" in args else f"0x00 ({tag}) 0x1000"
                self.assertEqual(inspect_elf(Path("synthetic-elf"), output)["glibc_floor"], "2.36")

    def test_unknown_private_and_malformed_versions_fail_closed(self):
        for tag in ("PRIVATE", "ABI_FUTURE", "2.43-extra", "02.43", "2.65536", "2.43.0.1", "2"):
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                self.inspect(["2.2.5", tag])

    def test_hashes_and_floor_cover_every_relocated_file(self):
        with tempfile.TemporaryDirectory(prefix="telorgon-abi-test-") as directory:
            stage = Path(directory)
            elf = bytearray(64)
            elf[:6] = b"\x7fELF\x02\x01"
            elf[18:20] = b"\x3e\x00"
            paths = ("bin/Xwayland", "bin/xkbcomp", "lib/libXau.so.6")
            for relative in paths:
                file = stage / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_bytes(elf + b"relocated $ORIGIN")
            def output(*args):
                if "--version-info" in args:
                    return "Name: GLIBC_" + ("2.43" if Path(args[-1]).name == "libXau.so.6" else "2.2.5")
                return " (NEEDED) Shared library: [libc.so.6]"
            policy = tomllib.loads(Path(__file__).with_name("runtime-policy.toml").read_text())
            audit = create_audit(stage, paths, policy, source_lock(REPOSITORY), output)
            self.assertEqual(audit["minimum_glibc"], "2.43")
            self.assertEqual(set(audit["files"]), set(paths))
            for relative in paths:
                self.assertEqual(audit["files"][relative]["sha256"], hashlib.sha256((stage / relative).read_bytes()).hexdigest())


if __name__ == "__main__":
    unittest.main()
