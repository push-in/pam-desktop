#!/usr/bin/env python3
"""Verify the portable host archive without trusting archive paths."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import sys
import tarfile


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"pam-desktop portable verifier: {message}")


def main() -> None:
    if len(sys.argv) != 3:
        fail("usage: verify-host-portable.py <archive> <target>")
    source, target = Path(sys.argv[1]), sys.argv[2]
    checksum = Path(f"{source}.sha256")
    expected = checksum.read_text(encoding="ascii").split()[0].lower()
    actual = hashlib.sha256(source.read_bytes()).hexdigest()
    if len(expected) != 64 or expected != actual:
        fail("archive checksum mismatch")
    expected_files = {"LICENSE", "bin/pam-desktop", "bin/pam-desktop-launcher", "manifest.json"}
    with tarfile.open(source, "r:gz") as archive:
        members = archive.getmembers()
        roots = {PurePosixPath(member.name).parts[0] for member in members}
        if len(roots) != 1:
            fail("archive must contain one package root")
        root = roots.pop()
        files: dict[str, bytes] = {}
        for member in members:
            path = PurePosixPath(member.name)
            if path.is_absolute() or ".." in path.parts or not member.isfile():
                fail(f"unsafe archive member {member.name}")
            relative = str(PurePosixPath(*path.parts[1:]))
            stream = archive.extractfile(member)
            if stream is None:
                fail(f"cannot read {member.name}")
            files[relative] = stream.read()
    if set(files) != expected_files:
        fail("archive file set does not match the portable host contract")
    manifest = json.loads(files["manifest.json"])
    if (
        manifest.get("schemaVersion") != 1
        or manifest.get("apiVersion") != 1
        or manifest.get("protocolVersion") != 6
        or manifest.get("target") != target
        or not re.fullmatch(r"\d+\.\d+\.\d+(?:[+-][0-9A-Za-z.-]+)?", manifest.get("version", ""))
    ):
        fail("manifest contract mismatch")
    declared = {entry["path"]: entry for entry in manifest.get("files", [])}
    for relative in expected_files - {"manifest.json"}:
        entry = declared.get(relative)
        payload = files[relative]
        if not entry or entry.get("bytes") != len(payload) or entry.get("sha256") != hashlib.sha256(payload).hexdigest():
            fail(f"manifest digest mismatch for {relative}")
    print(f"[ok] Portable host archive contract passed: {source}")


if __name__ == "__main__":
    main()
