"""Static ELF ABI inspection and the hash-bound staging audit contract.

Only inspection tools are launched; payload executables are never run. This
audit covers linked requirements of owned files, not host drivers or dlopen.
"""
import hashlib
import json
from pathlib import Path
import re
import sys

REPOSITORY = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPOSITORY / "tools/sdk"))
from native_inputs import glibc_version, locked_sources

AUDIT_PATH = "licenses/static-closure.json"


def is_elf(raw):
    return len(raw) >= 64 and raw[:6] == b"\x7fELF\x02\x01" and raw[18:20] == b"\x3e\x00"


def maximum_floor(floors):
    versions = [floor for floor in floors if floor is not None]
    if not versions:
        raise ValueError("ELF audit found no glibc requirements")
    return max(versions, key=glibc_version)


def inspect_elf(path, output):
    text = output("readelf", "--version-info", path)
    floors = []
    for tag in sorted(set(re.findall(r"\bGLIBC_([^\s()\[\]]+)", text))):
        if tag == "ABI_DT_RELR":
            # glibc added this version node with DT_RELR support in 2.36.
            floors.append("2.36")
        elif re.fullmatch(r"[0-9.]+", tag):
            glibc_version(tag)
            floors.append(tag)
        else:
            raise ValueError(f"unsupported glibc ABI requirement GLIBC_{tag}: {path}")
    dynamic = output("readelf", "-d", path)
    if re.search(r"\((?:RELR|RELRSZ|RELRENT)\)", dynamic):
        # Linkers can emit packed relocations without the GLIBC version node.
        floors.append("2.36")
    needed = re.findall(r"\(NEEDED\).*\[(.*?)\]", dynamic)
    return dict(needed=needed, glibc_floor=maximum_floor(floors) if floors else None)


def create_audit(destination, paths, policy, lock, output):
    files = {}
    for relative in sorted(paths):
        path = destination / relative
        raw = path.read_bytes()
        if not is_elf(raw):
            raise ValueError(f"expected x86-64 ELF file: {relative}")
        files[relative] = dict(sha256=hashlib.sha256(raw).hexdigest(), **inspect_elf(path, output))
    xwayland = locked_sources(REPOSITORY)["xwayland"]
    audit = dict(schema=1, target=lock["target"],
                 minimum_glibc=maximum_floor(record["glibc_floor"] for record in files.values()),
                 xwayland_version=xwayland["version"], xwayland_source_sha256=xwayland["sha256"],
                 files=files)
    validate_audit(audit, {relative: {"sha256": record["sha256"]}
                           for relative, record in files.items()}, policy, lock, xwayland)
    return audit


def decode_audit(raw):
    def unique_fields(pairs):
        fields = {}
        for key, value in pairs:
            if key in fields:
                raise ValueError(f"duplicate ELF audit field: {key}")
            fields[key] = value
        return fields
    return json.loads(raw, object_pairs_hook=unique_fields)


def validate_audit(audit, owned, policy, lock, xwayland):
    expected_fields = {"schema", "target", "minimum_glibc", "xwayland_version",
                       "xwayland_source_sha256", "files"}
    if not isinstance(audit, dict) or set(audit) != expected_fields or type(audit["schema"]) is not int or audit["schema"] != 1:
        raise ValueError("invalid or obsolete ELF audit schema; restage the payload")
    if audit["target"] != lock["target"] or audit["target"] != policy["target"]:
        raise ValueError("ELF audit target does not match source lock and runtime policy")
    if (audit["xwayland_version"], audit["xwayland_source_sha256"]) != (xwayland["version"], xwayland["sha256"]):
        raise ValueError("ELF audit Xwayland provenance does not match the source lock")
    glibc_version(audit["minimum_glibc"])
    files = audit["files"]
    if not isinstance(files, dict) or set(files) != set(owned):
        raise ValueError("ELF audit must cover every staged bin/lib file exactly")
    host, private = set(policy["host"]), set(policy["private"])
    if host & private:
        raise ValueError("libraries cannot be both host and private")
    for relative, record in files.items():
        if not isinstance(record, dict) or set(record) != {"sha256", "needed", "glibc_floor"}:
            raise ValueError(f"invalid ELF audit record: {relative}")
        if record["sha256"] != owned[relative]["sha256"]:
            raise ValueError(f"stale ELF audit hash: {relative}; restage the payload")
        if relative.startswith("lib/") and relative[4:] not in private:
            raise ValueError(f"undeclared private library in ELF audit: {relative}")
        if record["glibc_floor"] is not None:
            glibc_version(record["glibc_floor"])
        needed = record["needed"]
        if not isinstance(needed, list) or len(needed) > 256 or not all(isinstance(name, str) for name in needed):
            raise ValueError(f"invalid ELF dependencies: {relative}")
        for soname in needed:
            if soname in host:
                continue
            if soname not in private or f"lib/{soname}" not in files:
                raise ValueError(f"missing or undeclared private dependency {soname}: {relative}")
    floor = maximum_floor(record["glibc_floor"] for record in files.values())
    if audit["minimum_glibc"] != floor:
        raise ValueError("ELF audit minimum_glibc does not match its file requirements")
    return floor
