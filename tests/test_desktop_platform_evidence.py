import json
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts" / "desktop-platform-evidence.py"


class DesktopPlatformEvidenceTests(unittest.TestCase):
    def fixture(self, directory: Path) -> Path:
        binary = directory / "pam-desktop"
        binary.write_text("#!/bin/sh\nprintf 'pam-desktop 1.2.1\\n'\n", encoding="utf-8")
        binary.chmod(binary.stat().st_mode | stat.S_IXUSR)
        return binary

    def run_script(self, *arguments: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [str(SCRIPT), *arguments],
            cwd=ROOT,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_creates_and_verifies_bounded_integer_coded_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as raw_directory:
            directory = Path(raw_directory)
            binary = self.fixture(directory)
            evidence = directory / "evidence.json"
            created = self.run_script(
                "create",
                "--binary",
                str(binary),
                "--platform-code",
                "2",
                "--architecture-code",
                "1",
                "--revision",
                "a" * 40,
                "--rust-version",
                "rustc 1.88.0 (fixture)",
                "--output",
                str(evidence),
            )
            self.assertEqual(created.returncode, 0, created.stderr)
            report = json.loads(evidence.read_text(encoding="utf-8"))
            self.assertEqual(report["resultCode"], 1)
            self.assertEqual(report["suiteCode"], 3)
            self.assertEqual(report["surfaceCode"], 3)
            self.assertEqual(report["platformCode"], 2)
            self.assertEqual(report["architectureCode"], 1)
            self.assertNotIn(str(directory), evidence.read_text(encoding="utf-8"))
            verified = self.run_script(
                "verify", "--binary", str(binary), "--evidence", str(evidence)
            )
            self.assertEqual(verified.returncode, 0, verified.stderr)

    def test_rejects_binary_tampering_and_unknown_fields(self) -> None:
        with tempfile.TemporaryDirectory() as raw_directory:
            directory = Path(raw_directory)
            binary = self.fixture(directory)
            evidence = directory / "evidence.json"
            self.assertEqual(
                self.run_script(
                    "create",
                    "--binary",
                    str(binary),
                    "--platform-code",
                    "3",
                    "--architecture-code",
                    "2",
                    "--revision",
                    "b" * 40,
                    "--rust-version",
                    "rustc 1.88.1 (fixture)",
                    "--output",
                    str(evidence),
                ).returncode,
                0,
            )
            binary.write_text("tampered", encoding="utf-8")
            self.assertNotEqual(
                self.run_script(
                    "verify", "--binary", str(binary), "--evidence", str(evidence)
                ).returncode,
                0,
            )
            binary = self.fixture(directory)
            report = json.loads(evidence.read_text(encoding="utf-8"))
            report["unexpected"] = True
            evidence.write_text(json.dumps(report), encoding="utf-8")
            self.assertNotEqual(
                self.run_script(
                    "verify", "--binary", str(binary), "--evidence", str(evidence)
                ).returncode,
                0,
            )

    def test_rejects_an_uncertified_platform_architecture_pair(self) -> None:
        with tempfile.TemporaryDirectory() as raw_directory:
            directory = Path(raw_directory)
            result = self.run_script(
                "create",
                "--binary",
                str(self.fixture(directory)),
                "--platform-code",
                "2",
                "--architecture-code",
                "2",
                "--revision",
                "c" * 40,
                "--rust-version",
                "rustc 1.88.0 (fixture)",
                "--output",
                str(directory / "evidence.json"),
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("certified target", result.stderr)


if __name__ == "__main__":
    unittest.main()
