#!/usr/bin/env python3
"""Create or verify bounded PAM Desktop runtime performance evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import sys
from pathlib import Path


def fail(message: str) -> None:
    raise ValueError(message)


def integer(value: object, name: str, *, positive: bool = True) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        fail(f"{name} must be an integer")
    if positive and value <= 0:
        fail(f"{name} must be positive")
    return value


def canonical(value: object) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def load_object(path: Path) -> dict[str, object]:
    if path.stat().st_size > 1024 * 1024:
        fail(f"{path} exceeds 1 MiB")
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        fail(f"{path} must contain a JSON object")
    return value


def create(snapshot_path: Path, output: Path, idle_cpu_basis_points: int, revision: str) -> None:
    snapshot = load_object(snapshot_path)
    if snapshot.get("schemaVersion") != 1 or snapshot.get("surfaceCode") != 3:
        fail("snapshot is not a PAM Desktop diagnostics envelope")
    if snapshot.get("performanceComplete") is not True:
        fail("runtime performance samples are incomplete")
    if snapshot.get("performancePassed") is not True:
        fail("runtime performance budget failed")
    budget = snapshot.get("performanceBudget")
    if not isinstance(budget, dict):
        fail("snapshot performance budget is missing")
    idle_budget = integer(budget.get("idleCpuBasisPoints"), "idle CPU budget")
    idle_cpu_basis_points = integer(idle_cpu_basis_points, "idle CPU basis points", positive=False)
    if idle_cpu_basis_points < 0 or idle_cpu_basis_points > 10_000:
        fail("idle CPU basis points must be between 0 and 10,000")
    if idle_cpu_basis_points > idle_budget:
        fail("idle CPU budget failed")
    metrics = {
        "startup_milliseconds": integer(snapshot.get("startupMilliseconds"), "startup"),
        "resident_memory_bytes": integer(snapshot.get("residentMemoryBytes"), "resident memory"),
        "ipc_p95_microseconds": integer(snapshot.get("ipcP95Microseconds"), "IPC p95"),
        "frame_p95_microseconds": integer(snapshot.get("frameP95Microseconds"), "frame p95"),
        "idle_cpu_basis_points": idle_cpu_basis_points,
    }
    startup_snapshot_hit = snapshot.get("startupSnapshotHit")
    if not isinstance(startup_snapshot_hit, bool):
        fail("startup snapshot hit state is missing")
    revision = revision.lower()
    if len(revision) != 40 or any(character not in "0123456789abcdef" for character in revision):
        fail("source revision must be a 40-digit hexadecimal commit")
    evidence: dict[str, object] = {
        "schema_version": 1,
        "suite_id": 4,
        "surface_code": 3,
        "result_code": 1,
        "source_revision": revision,
        "environment": {
            "operating_system": platform.system(),
            "architecture": platform.machine(),
            "python": platform.python_version(),
        },
        "metrics": metrics,
        "startup_snapshot_hit": startup_snapshot_hit,
        "budget": budget,
        "snapshot_sha256": hashlib.sha256(canonical(snapshot)).hexdigest(),
    }
    evidence["evidence_sha256"] = hashlib.sha256(canonical(evidence)).hexdigest()
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.{os.getpid()}.tmp")
    temporary.write_bytes(json.dumps(evidence, indent=2, sort_keys=True).encode() + b"\n")
    os.replace(temporary, output)


def verify(path: Path) -> None:
    evidence = load_object(path)
    digest = evidence.pop("evidence_sha256", None)
    if not isinstance(digest, str) or digest != hashlib.sha256(canonical(evidence)).hexdigest():
        fail("performance evidence digest mismatch")
    if [evidence.get(key) for key in ("schema_version", "suite_id", "surface_code", "result_code")] != [1, 4, 3, 1]:
        fail("performance evidence contract mismatch")
    metrics = evidence.get("metrics")
    if not isinstance(evidence.get("startup_snapshot_hit"), bool):
        fail("performance evidence startup snapshot state is missing")
    if not isinstance(metrics, dict) or set(metrics) != {
        "startup_milliseconds", "resident_memory_bytes", "ipc_p95_microseconds",
        "frame_p95_microseconds", "idle_cpu_basis_points",
    }:
        fail("performance evidence metrics are incomplete")
    for name, value in metrics.items():
        integer(value, name, positive=name != "idle_cpu_basis_points")


def main() -> int:
    parser = argparse.ArgumentParser()
    subcommands = parser.add_subparsers(dest="command", required=True)
    create_parser = subcommands.add_parser("create")
    create_parser.add_argument("--snapshot", type=Path, required=True)
    create_parser.add_argument("--output", type=Path, required=True)
    create_parser.add_argument("--idle-cpu-basis-points", type=int, required=True)
    create_parser.add_argument("--revision", required=True)
    verify_parser = subcommands.add_parser("verify")
    verify_parser.add_argument("--evidence", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        if arguments.command == "create":
            create(arguments.snapshot, arguments.output, arguments.idle_cpu_basis_points, arguments.revision)
        else:
            verify(arguments.evidence)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"desktop performance evidence: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
