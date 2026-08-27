# Performance engineering

Pam Desktop performance claims must be reproducible. Compare release builds on
the same machine, desktop session, application fixture and measurement window.
Record the kernel, CPU governor, Servo/PAM versions and whether caches are warm.

When `startupSnapshot` is enabled, the host persists a private, atomically
replaced bootstrap snapshot in the operating-system cache directory. A snapshot
is accepted only when its schema, PAM protocol, project fingerprint and complete
bootstrap validation all match. The fingerprint covers Composer manifests and
project PHP sources while deliberately excluding generated builds and `vendor`;
`composer.lock` is the dependency identity. `pam-desktop doctor` reports
`warm hit` or `cold/miss`, and the development ready event carries the same fact
as `startupSnapshotHit`. A cache hit is evidence, not permission to skip the live
PHP worker handshake: the live contract remains authoritative and a mismatch
atomically replaces the snapshot.

The runtime diagnostics envelope also records `startupSnapshotHit`. Cold misses
are checked against `coldStartMilliseconds`; validated warm hits are checked
against the independently configured `warmStartMilliseconds` and report a
`warm-start` violation. A warm run can therefore never hide behind the looser
cold-start budget.

## Required measurements

| Metric | Definition |
| --- | --- |
| Cold start | Process spawn to first interactive bridge call |
| Warm start | Same measurement after filesystem cache warm-up |
| Idle RSS | Total proportional resident memory after 30 idle seconds |
| Window RSS | Incremental memory for a second equivalent window |
| Bridge latency | p50/p95/p99 of a no-op command over 10,000 calls |
| Parallel throughput | Completed independent commands per second |
| Database throughput | Prepared inserts and bounded reads per second |
| Bundle bytes | Compressed archive and installed directory sizes |
| Idle CPU | Process-tree CPU over a 60-second idle window |

Never compare a debug PAM binary against a packaged Electron application. Use
the release profile, which enables thin LTO, one codegen unit, abort-on-panic
and symbol stripping. PAM keeps the parallel pool lazy, initializes clipboard
only on first use and compiles SQLite into the host for reproducible bundles.

## Regression gates

Store benchmark JSON as CI artifacts and fail only against a rolling baseline,
not one noisy run. Recommended initial gates are a 10% cold-start regression,
15% idle-memory regression, 10% bridge p95 regression or 5% bundle-size
regression. Confirm any failure with at least five samples before blocking a
release.

Use `cargo bloat`, release flamegraphs and allocator/procfs samples to explain
regressions. An optimization is not complete until it has an observable metric
and a test or gate that keeps it from silently disappearing.

Package footprint is deterministic and therefore does not need repeated noisy
samples. Every CI and release host archive produces a schema-1, suite-2 Desktop
evidence manifest containing compressed archive bytes, installed bytes, combined
host executable bytes, compression ratio, archive SHA-256, source revision and
build environment. Create and authenticate it with:

```bash
PAM_EVIDENCE_REVISION=$(git rev-parse HEAD) \
  scripts/desktop-footprint-evidence.sh create dist/pam-desktop-*.tar.gz footprint.json
scripts/desktop-footprint-evidence.sh verify dist/pam-desktop-*.tar.gz footprint.json
scripts/desktop-footprint-evidence.sh compare footprint.json previous-footprint.json 5
```

The comparison command applies the percentage ceiling independently to archive,
installed and executable bytes. CI consumers should download the last accepted
release manifest as the baseline; missing baseline evidence must be reported as
“not compared”, never interpreted as a pass. Cold start, RSS and idle CPU still
require a real graphical session and remain separate from this deterministic
package gate.

macOS arm64 and Windows x64 pre-release jobs additionally emit schema-1,
suite-3 Desktop host evidence. Platform codes are `2` (macOS) and `3`
(Windows); architecture codes are `1` (arm64) and `2` (x86-64). The manifest
records the exact source commit, Rust 1.88 toolchain, executable byte count and
SHA-256, then reproduces the bounded `--version` handshake during verification.
Only the compact evidence is uploaded and it expires after 14 days; the large
debug executable dies with the disposable runner so cross-platform proof does
not become an unbounded build archive. This is startup/linkage evidence only;
cold start, RSS, accessibility and renderer readiness still require a real
graphical session.

The authenticated `pam.diagnostics.snapshot()` API supplies worker generation,
active/failed command counters and aggregate latency to the in-app inspector.
See [Runtime diagnostics](diagnostics.md).

The checked-in `benchmarks/desktop/bridge.js` harness produces the bridge
latency/throughput JSON. Its companion fixture documentation fixes warm-up and
sample counts so PAM, Electron and other runtimes can be measured under the
same workload. Framework names never substitute for measurements: publish raw
artifacts and machine metadata with every comparison.

The harness also provides `capturePamFrameBenchmark({samples: 600})`. The
runtime retains only the newest 2,048 IPC and frame observations and exposes
`startupMilliseconds`, `ipcP95Microseconds`, `frameP95Microseconds`,
`performancePassed`, and stable violation names from
`pam.diagnostics.snapshot()`. Missing observations remain `null`, never an
invented zero.

Release automation converts a complete diagnostics snapshot plus an externally
sampled idle CPU interval into schema-1, suite-4 evidence:

```bash
scripts/desktop-performance-evidence.py create \
  --snapshot runtime-snapshot.json \
  --idle-cpu-basis-points 50 \
  --revision "$(git rev-parse HEAD)" \
  --output performance-evidence.json
scripts/desktop-performance-evidence.py verify \
  --evidence performance-evidence.json
```

Creation fails when runtime samples are incomplete, any runtime budget failed,
idle CPU exceeds its declared budget, or the source revision is not exact.
Verification authenticates the canonical evidence digest and rejects missing,
renamed or non-integer metrics.
