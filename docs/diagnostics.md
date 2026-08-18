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
host-observed command time in microseconds; the primary worker generation;
active parallel worker count; and the current event cursor.

Worker generation increments after timeout, cancellation, crash or runtime hot
reload. A rising generation paired with failures is therefore immediately
visible to application diagnostics. The average is intentionally aggregate and
bounded: use the browser Performance API around individual calls when a local
development build needs command-level timings.

Production applications may expose this data in a support panel. It contains
operational counters only and is available solely to the authenticated local
origin through the ephemeral bridge token.
