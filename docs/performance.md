# Performance engineering

Pam Desktop performance claims must be reproducible. Compare release builds on
the same machine, desktop session, application fixture and measurement window.
Record the kernel, CPU governor, Servo/PAM versions and whether caches are warm.

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

The authenticated `pam.diagnostics.snapshot()` API supplies worker generation,
active/failed command counters and aggregate latency to the in-app inspector.
See [Runtime diagnostics](diagnostics.md).

The checked-in `benchmarks/desktop/bridge.js` harness produces the bridge
latency/throughput JSON. Its companion fixture documentation fixes warm-up and
sample counts so PAM, Electron and other runtimes can be measured under the
same workload. Framework names never substitute for measurements: publish raw
artifacts and machine metadata with every comparison.
