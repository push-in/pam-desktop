# Architecture

Pam Desktop keeps policy in PHP and dangerous capabilities at a narrow Rust
boundary. The browser surface never receives a generic process, filesystem, or
shell API.

## Process model

```text
trusted local UI
HTML / CSS / JavaScript
        │ invoke / emit / events / cancel
        │ origin + ephemeral token
        ▼
Rust host
Winit ─ Servo ─ loopback gateway
  │                  │
  │ typed effects    │ bounded JSON lines
  │                  ▼
  └──────────── Pam worker
                  PHP 8.4
               pam/desktop app
```

`pam desktop dev .` delegates to the internal host, which performs these steps:

1. Resolve the project root and require `app.php`, `composer.json`, and
   `vendor/autoload.php`.
2. Start `pam exec app.php` with piped standard input and output.
3. Send the reserved `@pam/boot` request and validate all returned window,
   entry and command-timeout contracts.
4. Bind an ephemeral port on `127.0.0.1`, generate a 256-bit bridge token, and
   start the local gateway.
5. Create one native Winit window, rendering context and Servo WebView for each
   declared application window.
6. Navigate each WebView to its isolated gateway route and process targeted
   effects until the main window exits.
7. Gracefully stop the gateway and terminate the supervised PHP worker.

The PHP worker is long-lived. Registered commands therefore retain application
state across invocations, while the UI and host stay responsive in separate
execution contexts.

## Ownership

| Layer | Owns | Does not own |
| --- | --- | --- |
| Pam core | PHP Embed runtime and `pam exec` | Servo, windows, frontend assets |
| PHP package | windows, commands, events, deadlines, typed effects | operating-system handles |
| Protocol crate | envelopes, versions, integer enums, validation | application commands |
| Rust shell | process lifecycle, gateway, Winit, Servo, effects | domain logic |
| Frontend | presentation and explicit command calls | ambient native capabilities |

This division lets Pam Desktop evolve independently without coupling Servo's
native dependency graph to every Pam server installation.

## Protocol

Host and worker exchange one JSON object per line. Messages are limited to
1 MiB and correlated by monotonically increasing unsigned request IDs.

All coded variants are sequential integers:

| Field | Values |
| --- | --- |
| message kind | `1` request, `2` response |
| response status | `1` success, `2` failure |
| window theme | `1` system, `2` light, `3` dark |
| effect kind | `1` title, `2` visibility, `3` close, `4` focus |
| error code | `1` through `12`, defined by `ErrorCode` |

Command names are application-owned strings because they are identifiers, not
stored domain variants. They must begin with a letter, contain only ASCII
letters, digits, dots, dashes or underscores, and remain at most 64 bytes.

Protocol 2 adds a source `windowId`, response events, multiple boot windows and
the command deadline. The version is validated on every response. A mismatched
version, message kind, request ID, malformed payload, oversized line, or failure
without an error stops that invocation explicitly.

## Bridge security

The gateway is a transport boundary, not a public web server:

- it binds only to a random `127.0.0.1` port;
- every process receives a cryptographically random 32-byte token;
- command requests require both the exact `Origin` and token;
- tokens are compared without an early content exit;
- assets are canonicalized and must remain below the selected public root;
- navigation outside the gateway origin is denied by the Servo delegate;
- responses set `no-store`, `nosniff`, and a restrictive Content Security Policy;
- the injected API is frozen and exposes only `invoke`, `emit`, `on` and the
  current `windowId`.

Frontend assets are trusted application code. Loading arbitrary remote pages
inside the privileged view is intentionally unsupported. New native powers
should be introduced as named, typed capabilities with validation on both sides,
not as arbitrary code execution.

## Concurrency and failure

Protocol 2 serializes commands through one worker mutex. This gives
deterministic PHP state and avoids pretending that a single Zend runtime is
parallel. Axum handles transport concurrently, while blocking worker I/O runs
outside Tokio's async workers.

Each request has a bounded deadline and a cancellation token. If a deadline
expires, the caller aborts, the PHP worker exits, emits invalid output, or
exceeds the message limit, the host kills and reaps that worker and starts a
fresh generation. The interrupted command is never retried automatically
because it may have completed a side effect just before failing. The next
request uses the recovered worker.

The event hub retains a bounded ordered history and filters targeted events by
window. Long polling does not block Tokio workers. The project watcher classifies
asset changes separately from PHP/Composer changes: assets reload all WebViews;
runtime changes restart the worker, validate a new bootstrap and atomically
reconfigure windows. Invalid reloads keep the host alive and emit
`pam.dev.error`.

## Extension rules

When adding a capability:

1. Model every status, kind, type, or other coded variant as a sequential
   integer enum beginning at `1`.
2. Add the Rust protocol contract and PHP enum together.
3. Validate untrusted payloads before reaching operating-system APIs.
4. Return a typed effect or command result; never expose a generic shell bridge.
5. Add round-trip tests and document whether the capability is synchronous,
   cancellable, persistent, and platform-specific.
6. Bump the protocol version for incompatible envelope or semantic changes.
