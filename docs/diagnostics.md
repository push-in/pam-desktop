# Runtime diagnostics

The frozen `pam.diagnostics` namespace exposes aggregate host measurements for
an in-app inspector without leaking command payloads, filesystem paths, SQL,
user identity or bridge credentials.

```js
const runtime = await pam.diagnostics.snapshot();
```

During `pam desktop dev`, open `/_pam/inspector` on the printed local gateway
origin for a responsive, keyboard-accessible live dashboard. It refreshes once
per second, offers an explicit refresh control, announces connection state to
assistive technology, respects reduced motion and is not served by production
`run` builds.

The same live snapshot is available to terminal tooling while the development
host is running:

```bash
pam desktop diagnostics .
# From the PAM Desktop project root, the unified CLI also accepts:
pam diagnostics
```

The development host publishes an ephemeral `.pam/desktop-session.json`
descriptor so the CLI can authenticate to the loopback gateway. The descriptor
is capped at 8 KiB, restricted to the current user (`0700` directory and `0600`
file on Unix), rejected when it is a symlink, and removed at shutdown only when
its process and token still match. It contains no application payloads. Keep
`.pam/` ignored and restart `pam dev` if an unclean shutdown leaves a stale
descriptor. Production `run` builds never publish one.

The snapshot contains total, failed and currently active PHP commands; average
and rolling p95 host-observed command time; the primary worker generation;
active parallel worker count; current event cursor; host uptime, first-command
startup time and resident memory; open PTY sessions; active Rust plugin count;
and rolling frame p95. The visual Runtime Inspector renders these counters and
the configured Workstation budget result live.

Worker generation increments after timeout, cancellation, crash or runtime hot
reload. A rising generation paired with failures is therefore immediately
visible to application diagnostics. The average is intentionally aggregate and
bounded: use the browser Performance API around individual calls when a local
development build needs command-level timings.

Command and frame percentiles use bounded 2,048-sample rolling windows, so a
long session cannot grow diagnostic memory. A renderer records a measured frame
interval with `pam.diagnostics.reportFrame(frameMicroseconds)`. The benchmark
fixture exposes `capturePamFrameBenchmark()` for repeatable
`requestAnimationFrame` sampling. Missing observations remain `null` and render
as `collecting`; they are never silently treated as zero-latency evidence.

Production applications may expose this data in a support panel. It contains
operational counters only and is available solely to the authenticated local
origin through the ephemeral bridge token.

## Native crash reports

When the workstation crash-report policy is enabled, the host installs a panic
hook before creating the event loop. A native panic writes an atomic schema-1
JSON report under the operating system's per-user state directory. Reports
contain the application and host versions, process/thread identity, source
location and a bounded symbolizable Rust backtrace. They never include bridge
tokens, command payloads, clipboard data, SQL, document contents or secrets.

Only the eight newest reports are retained. Unix reports are created with mode
`0600`; applications decide explicitly whether and where to upload them.

## Opt-in OTLP command traces

The host can export one root span per Desktop command over OTLP HTTP/JSON. It
is disabled by default and an endpoint alone never enables it. Set all three:

```bash
PAM_DESKTOP_OTLP_ENABLED=true
OTEL_EXPORTER_OTLP_TRACES_PROTOCOL=http/json
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=https://collector.example/v1/traces
```

Remote endpoints must use HTTPS; plain HTTP is accepted only for loopback
development collectors. Standard OTLP headers, timeout, batch size, queue size
and schedule delay environment variables are supported. Redirects are refused,
responses are capped at 64 KiB, and export runs on a bounded non-blocking queue.

Every `pam.desktop.command` span contains only the static command name, integer
execution lane, integer outcome, service name/version, timestamps and OTLP
status. Payloads, results, window/request identifiers, origins, filesystem
paths, user identity and bridge credentials are never recorded. Diagnostics
report exported, dropped, rejected and failed-export totals so backpressure and
Collector partial success remain visible.

An authenticated application invocation may continue a server trace by passing
the exact W3C version `00` context returned by that server:

```js
await pam.invoke("catalog.refresh", null, {
  traceparent: response.headers.get("traceparent"),
});
```

The bridge sends this field only with its exact local origin and ephemeral
256-bit token. The Rust host validates lowercase hexadecimal, field lengths and
nonzero trace/span identifiers before command execution. A valid context keeps
the trace ID and sampling flags, uses its span ID as the Desktop parent, and
still creates a distinct Desktop span ID. Invalid contexts fail with a bounded
client error; they are never normalized or exported. `tracestate` is not
accepted because PAM does not yet have a vendor allowlist or size policy for it.
