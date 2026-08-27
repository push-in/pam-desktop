import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "desktop-performance-evidence.py"


class PerformanceEvidenceTest(unittest.TestCase):
    def test_create_verify_and_reject_tampering(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            snapshot = root / "snapshot.json"
            evidence = root / "evidence.json"
            snapshot.write_text(json.dumps({
                "schemaVersion": 1,
                "surfaceCode": 3,
                "performanceComplete": True,
                "performancePassed": True,
                "startupMilliseconds": 120,
                "startupSnapshotHit": True,
                "residentMemoryBytes": 1024,
                "ipcP95Microseconds": 300,
                "frameP95Microseconds": 16000,
                "performanceBudget": {"idleCpuBasisPoints": 100},
            }))
            subprocess.run([
                "python3", str(SCRIPT), "create", "--snapshot", str(snapshot),
                "--output", str(evidence), "--idle-cpu-basis-points", "50",
                "--revision", "a" * 40,
            ], check=True)
            subprocess.run(["python3", str(SCRIPT), "verify", "--evidence", str(evidence)], check=True)
            value = json.loads(evidence.read_text())
            self.assertIs(value["startup_snapshot_hit"], True)
            value["metrics"]["ipc_p95_microseconds"] = 1
            evidence.write_text(json.dumps(value))
            self.assertNotEqual(subprocess.run([
                "python3", str(SCRIPT), "verify", "--evidence", str(evidence)
            ], check=False).returncode, 0)

    def test_rejects_incomplete_runtime_samples(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            snapshot = root / "snapshot.json"
            snapshot.write_text(json.dumps({"schemaVersion": 1, "surfaceCode": 3, "performanceComplete": False}))
            result = subprocess.run([
                "python3", str(SCRIPT), "create", "--snapshot", str(snapshot),
                "--output", str(root / "evidence.json"), "--idle-cpu-basis-points", "0",
                "--revision", "b" * 40,
            ], check=False)
            self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
