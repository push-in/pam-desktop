from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent


class PortableHostArchiveTest(unittest.TestCase):
    def test_builds_reproducibly_and_verifies_the_manifest(self) -> None:
        with tempfile.TemporaryDirectory(prefix="pam-portable-test-") as temporary:
            root = Path(temporary)
            binaries = root / "bin"
            binaries.mkdir()
            host = binaries / "pam-desktop"
            host.write_text(
                "#!/bin/sh\n[ \"$1\" = \"--version\" ] && printf 'pam-desktop 1.2.9\\n'\n",
                encoding="utf-8",
            )
            host.chmod(0o755)
            launcher = binaries / "pam-desktop-launcher"
            launcher.write_bytes(host.read_bytes())
            launcher.chmod(0o755)
            environment = {**os.environ, "SOURCE_DATE_EPOCH": "1700000000"}
            archives = []
            for name in ("one", "two"):
                output = root / name
                subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / "scripts/package-host-portable.py"),
                        str(binaries),
                        str(output),
                        "1.2.9",
                        "aarch64-apple-darwin",
                    ],
                    check=True,
                    env=environment,
                )
                archive = output / "pam-desktop-1.2.9-aarch64-apple-darwin.tar.gz"
                subprocess.run(
                    [
                        sys.executable,
                        str(ROOT / "scripts/verify-host-portable.py"),
                        str(archive),
                        "aarch64-apple-darwin",
                    ],
                    check=True,
                )
                self.assertEqual(
                    sorted(path.name for path in output.iterdir()),
                    [archive.name, f"{archive.name}.sha256"],
                )
                archives.append(archive)
            self.assertEqual(archives[0].read_bytes(), archives[1].read_bytes())

    def test_verifier_rejects_a_traversal_member(self) -> None:
        with tempfile.TemporaryDirectory(prefix="pam-portable-unsafe-") as temporary:
            root = Path(temporary)
            archive = root / "unsafe.tar.gz"
            payload = root / "payload"
            payload.write_text("unsafe", encoding="utf-8")
            with tarfile.open(archive, "w:gz") as stream:
                stream.add(payload, arcname="package/../outside")
            import hashlib

            Path(f"{archive}.sha256").write_text(
                f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n",
                encoding="ascii",
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(ROOT / "scripts/verify-host-portable.py"),
                    str(archive),
                    "aarch64-apple-darwin",
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("unsafe archive member", result.stderr)


if __name__ == "__main__":
    unittest.main()
