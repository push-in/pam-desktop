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
