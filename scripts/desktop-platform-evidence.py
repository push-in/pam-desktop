#!/usr/bin/env python3
"""Create and verify bounded PAM Desktop native-host smoke evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from enum import IntEnum
from pathlib import Path


class EvidenceResult(IntEnum):
    PASSED = 1


class EvidenceSuite(IntEnum):
    NATIVE_HOST_SMOKE = 3


class Surface(IntEnum):
    DESKTOP = 3


class Platform(IntEnum):
    MACOS = 2
    WINDOWS = 3


class Architecture(IntEnum):
    ARM64 = 1
    X86_64 = 2


COMMIT = re.compile(r"^[0-9a-f]{40}$")
VERSION_OUTPUT = re.compile(r"^pam-desktop [0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def require_integer_enum(value: object, enum: type[IntEnum], label: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{label} must be an integer")
    try:
        enum(value)
    except ValueError as error:
        raise ValueError(f"{label} is unsupported") from error
    return value


def create(arguments: argparse.Namespace) -> None:
    binary = Path(arguments.binary).resolve(strict=True)
    if not binary.is_file():
        raise ValueError("desktop host must be a regular file")
    platform_code = require_integer_enum(arguments.platform_code, Platform, "platformCode")
    architecture_code = require_integer_enum(
        arguments.architecture_code,
        Architecture,
        "architectureCode",
    )
    if (platform_code, architecture_code) not in {
        (Platform.MACOS, Architecture.ARM64),
        (Platform.WINDOWS, Architecture.X86_64),
    }:
        raise ValueError("platform and architecture do not identify a certified target")
    if not COMMIT.fullmatch(arguments.revision):
        raise ValueError("revision must be a lowercase 40-character Git commit")
    rust_version = arguments.rust_version.strip()
    if not rust_version.startswith("rustc 1.88.") or "\n" in rust_version:
        raise ValueError("rustVersion must identify the supported Rust 1.88 toolchain")
    completed = subprocess.run(
        [str(binary), "--version"],
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )
    smoke_output = completed.stdout.strip()
    if completed.returncode != 0 or VERSION_OUTPUT.fullmatch(smoke_output) is None:
        raise ValueError("desktop host did not pass the bounded --version smoke test")
    report = {
        "schemaVersion": 1,
        "resultCode": EvidenceResult.PASSED,
        "suiteCode": EvidenceSuite.NATIVE_HOST_SMOKE,
        "surfaceCode": Surface.DESKTOP,
        "platformCode": platform_code,
        "architectureCode": architecture_code,
        "sourceCommit": arguments.revision,
        "rustVersion": rust_version,
        "binary": {
            "name": binary.name,
            "bytes": binary.stat().st_size,
            "sha256": digest(binary),
        },
        "smoke": {
            "argumentCode": 1,
            "output": smoke_output,
        },
    }
    output = Path(arguments.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


def verify(arguments: argparse.Namespace) -> None:
    binary = Path(arguments.binary).resolve(strict=True)
    report = json.loads(Path(arguments.evidence).read_text(encoding="utf-8"))
    if set(report) != {
        "schemaVersion",
        "resultCode",
        "suiteCode",
        "surfaceCode",
        "platformCode",
        "architectureCode",
        "sourceCommit",
        "rustVersion",
        "binary",
        "smoke",
    }:
        raise ValueError("evidence contains an incomplete or unknown top-level field")
    if (
        report["schemaVersion"] != 1
        or report["resultCode"] != EvidenceResult.PASSED
        or report["suiteCode"] != EvidenceSuite.NATIVE_HOST_SMOKE
        or report["surfaceCode"] != Surface.DESKTOP
    ):
        raise ValueError("evidence identity is invalid")
    platform_code = require_integer_enum(report["platformCode"], Platform, "platformCode")
    architecture_code = require_integer_enum(
        report["architectureCode"], Architecture, "architectureCode"
    )
    if (platform_code, architecture_code) not in {
        (Platform.MACOS, Architecture.ARM64),
        (Platform.WINDOWS, Architecture.X86_64),
    }:
        raise ValueError("platform and architecture do not identify a certified target")
    if not isinstance(report["sourceCommit"], str) or COMMIT.fullmatch(report["sourceCommit"]) is None:
        raise ValueError("sourceCommit is invalid")
    if not isinstance(report["rustVersion"], str) or not report["rustVersion"].startswith(
        "rustc 1.88."
    ):
        raise ValueError("rustVersion is invalid")
    binary_report = report["binary"]
    if not isinstance(binary_report, dict) or set(binary_report) != {"name", "bytes", "sha256"}:
        raise ValueError("binary evidence is invalid")
    actual_hash = digest(binary)
    if (
        binary_report["name"] != binary.name
        or binary_report["bytes"] != binary.stat().st_size
        or not isinstance(binary_report["sha256"], str)
        or SHA256.fullmatch(binary_report["sha256"]) is None
        or binary_report["sha256"] != actual_hash
    ):
        raise ValueError("binary identity does not match the evidence")
    smoke = report["smoke"]
    if (
        not isinstance(smoke, dict)
        or set(smoke) != {"argumentCode", "output"}
        or smoke["argumentCode"] != 1
        or not isinstance(smoke["output"], str)
        or VERSION_OUTPUT.fullmatch(smoke["output"]) is None
    ):
        raise ValueError("smoke evidence is invalid")
    completed = subprocess.run(
        [str(binary), "--version"],
        check=False,
        capture_output=True,
        text=True,
        timeout=20,
    )
    if completed.returncode != 0 or completed.stdout.strip() != smoke["output"]:
        raise ValueError("desktop host no longer reproduces the recorded smoke result")


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser()
    subcommands = result.add_subparsers(dest="command", required=True)
    create_parser = subcommands.add_parser("create")
    create_parser.add_argument("--binary", required=True)
    create_parser.add_argument("--platform-code", required=True, type=int)
    create_parser.add_argument("--architecture-code", required=True, type=int)
    create_parser.add_argument("--revision", required=True)
    create_parser.add_argument("--rust-version", required=True)
    create_parser.add_argument("--output", required=True)
    verify_parser = subcommands.add_parser("verify")
    verify_parser.add_argument("--binary", required=True)
    verify_parser.add_argument("--evidence", required=True)
    return result


def main() -> int:
    arguments = parser().parse_args()
    try:
        create(arguments) if arguments.command == "create" else verify(arguments)
    except (OSError, ValueError, json.JSONDecodeError, subprocess.SubprocessError) as error:
        print(f"desktop platform evidence: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
