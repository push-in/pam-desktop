#!/usr/bin/env python3
"""Build a deterministic PAM Desktop host archive on macOS or Windows."""

from __future__ import annotations

import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

TARGETS = {
    "aarch64-apple-darwin": "",
    "x86_64-apple-darwin": "",
    "x86_64-pc-windows-msvc": ".exe",
    "aarch64-pc-windows-msvc": ".exe",
}
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[+-][0-9A-Za-z.-]+)?$")
FILES = ("LICENSE", "bin/pam-desktop", "bin/pam-desktop-launcher")


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"pam-desktop portable packager: {message}")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def add_file(archive: tarfile.TarFile, root: Path, relative: str, epoch: int) -> None:
    path = root / relative
    info = archive.gettarinfo(str(path), arcname=f"{root.name}/{relative}")
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = epoch
    info.mode = 0o755 if relative.startswith("bin/") else 0o644
    with path.open("rb") as stream:
        archive.addfile(info, stream)


def main() -> None:
    if len(sys.argv) != 5:
        fail("usage: package-host-portable.py <binary-dir> <output-dir> <version> <target>")
    binary_dir, output_dir = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    version, target = sys.argv[3].removeprefix("v"), sys.argv[4]
    if not VERSION.fullmatch(version) or target not in TARGETS:
        fail("invalid version or unsupported native target")
    try:
        epoch = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    except ValueError:
        fail("SOURCE_DATE_EPOCH must be a non-negative integer")
    if epoch < 0:
        fail("SOURCE_DATE_EPOCH must be a non-negative integer")

    suffix = TARGETS[target]
    binaries = {
        "pam-desktop": binary_dir / f"pam-desktop{suffix}",
        "pam-desktop-launcher": binary_dir / f"pam-desktop-launcher{suffix}",
    }
    for name, path in binaries.items():
        if not path.is_file() or path.is_symlink():
            fail(f"missing or unsafe {name} binary: {path}")
    observed = subprocess.run(
        [str(binaries["pam-desktop"]), "--version"],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if observed != f"pam-desktop {version}":
        fail(f"host version mismatch: {observed!r}")

    package = f"pam-desktop-{version}-{target}"
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_path = output_dir / f"{package}.tar.gz"
    checksum_path = Path(f"{archive_path}.sha256")
    if archive_path.exists() or checksum_path.exists():
        fail(f"refusing to replace existing artifact {archive_path}")

    repository = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="pam-desktop-portable-") as temporary:
        root = Path(temporary) / package
        (root / "bin").mkdir(parents=True)
        shutil.copyfile(repository / "LICENSE", root / "LICENSE")
        for name, source in binaries.items():
            destination = root / "bin" / name
            shutil.copyfile(source, destination)
            destination.chmod(destination.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
        manifest_files = []
        for relative in FILES:
            path = root / relative
            manifest_files.append(
                {"path": relative, "bytes": path.stat().st_size, "sha256": digest(path)}
            )
        manifest = {
            "schemaVersion": 1,
            "apiVersion": 1,
            "protocolVersion": 6,
            "version": version,
            "target": target,
            "files": manifest_files,
        }
        (root / "manifest.json").write_text(
            json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n"
        )
        temporary_archive = Path(temporary) / "host.tar.gz"
        with temporary_archive.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=epoch, compresslevel=9) as compressed:
                with tarfile.open(fileobj=compressed, mode="w") as archive:
                    for relative in (*FILES, "manifest.json"):
                        add_file(archive, root, relative, epoch)
        os.replace(temporary_archive, archive_path)

    checksum_path.write_text(f"{digest(archive_path)}  {archive_path.name}\n", encoding="ascii")
    print(f"[ok] Portable host archive: {archive_path}")


if __name__ == "__main__":
    main()
