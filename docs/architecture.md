# Architecture

Pam Desktop keeps policy in PHP and dangerous capabilities at a narrow Rust
boundary. The browser surface never receives a generic process, filesystem, or
shell API.

## Process model

```text
trusted local UI
HTML / CSS / JavaScript
        │ invoke / events / named native APIs
        │ origin + ephemeral token
        ▼
Rust host
Winit ─ Servo ─ loopback gateway
  │         │            │
  │         │            ├── supervised Rust plugin processes
  │         │            │   versioned bounded JSON lines
  │         │            └── interruptible background scheduler
  │         │
  │ typed effects / bounded JSON lines
  ▼
Pam worker
PHP 8.4 + pam/desktop app
```

`pam desktop dev .` delegates to the internal host, which performs these steps:

1. Resolve the project root and require `app.php`, `composer.json`, and
   `vendor/autoload.php`.
2. Start `pam exec app.php` with piped standard input and output.
3. Send the reserved `@pam/boot` request and validate all returned window,
   application-manifest, entry, command-timeout, native-capability, native-shell,
   job and plugin contracts.
4. Bind an ephemeral port on `127.0.0.1`, generate a 256-bit bridge token, and
   start the local gateway.
5. Open each declared filesystem root once as a capability-scoped directory;
   no request receives ambient filesystem authority.
6. Start declared Rust plugin processes, validate their metadata handshake, and
   install the interruptible PHP job schedule.
7. Register menus, tray and global shortcuts, then create one native Winit
   window, rendering context and Servo WebView per application window.
8. Navigate each WebView to its isolated gateway route and process targeted
   window/shell effects until the main window exits.
9. Stop jobs and plugins, gracefully stop the gateway, and terminate the
   supervised PHP worker.

The PHP worker is long-lived. Registered commands therefore retain application
state across invocations, while the UI and host stay responsive in separate
execution contexts.

## Ownership

| Layer | Owns | Does not own |
| --- | --- | --- |
| Pam core | PHP Embed runtime and `pam exec` | Servo, windows, frontend assets |
| PHP package | windows, commands, events, jobs, plugins, shell policy, deadlines, capabilities, update policy, typed effects | operating-system handles and signing secrets |
| Protocol crate | envelopes, versions, integer enums, validation | application commands |
| Rust plugin SDK | isolated command handler and metadata contract | Servo or host memory |
| Rust shell | lifecycle, gateway, Servo, scheduler, plugin supervision, scoped filesystem, native adapters | domain logic |
| Frontend | presentation and explicit command calls | ambient native capabilities |

This division lets Pam Desktop evolve independently without coupling Servo's
native dependency graph to every Pam server installation.

## Stable 1.x boundary

Public API version `1`, worker protocol `6`, and Rust plugin protocol `1` are
independent compatibility axes. PHP exposes the first two as
`Application::API_VERSION` and `Application::PROTOCOL_VERSION`; the injected,
frozen frontend bridge exposes `window.pam.apiVersion`.

The 1.x compatibility suite reflects the complete PHP surface, round-trips
golden protocol messages and compiles an external-style Rust SDK consumer.
Protocol fixtures are values, not implementation snapshots: a field removal,
renaming, discriminator change or serialization-default change fails CI.

## Protocol

Host and worker exchange one JSON object per line. Messages are limited to
1 MiB and correlated by monotonically increasing unsigned request IDs.

All coded variants are sequential integers:

| Field | Values |
| --- | --- |
| message kind | `1` request, `2` response |
| response status | `1` success, `2` failure |
| window theme | `1` system, `2` light, `3` dark |
| effect kind | `1` title through `7` tray visibility |
| error code | `1` through `26`, defined by `ErrorCode` |
| file access | `1` read, `2` write, `3` read-write |
| file entry kind | `1` file, `2` directory |
| file operation | `1` read text through `5` create directory |
| dialog kind | `1` open file through `4` open directory |
| clipboard operation | `1` read, `2` write, `3` clear |
| notification urgency | `1` low, `2` normal, `3` critical |
| application category | `1` development through `8` education |
| update policy | `1` manual, `2` notify, `3` automatic |
| update platform | `1` Linux, `2` Windows, `3` macOS |
| update artifact kind | `1` portable, `2` native installer |
| update state | `1` disabled through `9` failed |
| menu item kind | `1` command, `2` checkbox, `3` separator, `4` submenu |
| tray close behavior | `1` exit, `2` hide |
| shortcut state | `1` pressed, `2` released |
| job overlap policy | `1` skip, `2` wait |

Command names are application-owned strings because they are identifiers, not
stored domain variants. They must begin with a letter, contain only ASCII
letters, digits, dots, dashes or underscores, and remain at most 64 bytes.

Protocol 6 adds native shell, background-job, PHP plugin and supervised Rust
plugin contracts. Rust plugins use a separate protocol version `1` so their
process boundary can evolve independently.
Protocol 5 added immutable update policy and signed-feed target contracts.
Protocol 4 introduced application identity and distribution metadata.
Protocol 3 introduced native-capability policy and the sequential
integer contracts for filesystem, dialogs, clipboard and notifications. The
version is validated on every response. A mismatched
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
- the injected API and each nested native API are frozen;
- native routes repeat origin, token and source-window validation;
- filesystem roots are opened through `cap-std` and accept only relative paths
  without parent components;
- final symbolic links are rejected, and capability-based traversal prevents
  intermediate links from escaping a root;
- selected and dropped resources become random 256-bit grants scoped to the
  current host process;
- dialogs return names, integer kinds, integer access and opaque grant IDs, not
  paths;
- clipboard ownership is retained by the host so copied content stays valid.

Frontend assets are trusted application code. Loading arbitrary remote pages
inside the privileged view is intentionally unsupported. New native powers
should be introduced as named, typed capabilities with validation on both sides,
not as arbitrary code execution.

## Concurrency and failure

Protocol 6 serializes commands, PHP events and background jobs through one
worker mutex. This gives
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

Hot-reloading PHP capability policy prepares a complete replacement native
service before it becomes visible. A successful replacement expires old file
grants and atomically swaps the authorized roots; invalid roots keep the current
application alive and emit `pam.dev.error`.

The same prepare-before-swap rule applies to plugins, jobs, menus, tray, and
shortcuts. Old schedules are cancelled and joined before replacement. A Rust
plugin crash, timeout, or cancellation terminates only that plugin process,
prepares a fresh instance for the next call, and never retries the interrupted
command.

## Distribution boundary

`pam desktop build` boots the same PHP application and validates the same
protocol contract used at runtime. It then stages the project in a random
directory under the selected output, materializes Composer package symlinks,
rejects non-vendor symlink escapes, omits secrets and configured exclusions,
and copies the exact `pam-desktop` and Pam worker binaries.

On Linux, `ldd` is applied only to those trusted binaries and non-glibc runtime
libraries are copied beside them. The launcher fixes `PAM_BINARY`,
bundle/update roots, `PHPRC`, `PHP_INI_SCAN_DIR` and the platform library path
before starting production `run` mode without the watcher.

The staged bundle receives platform metadata, validated icons and a sorted
integrity manifest. Linux adds Freedesktop metadata and scoped user installers.
Existing Windows/macOS packaging code is experimental and is neither generated
nor compatibility-tested for 1.x. Existing artifacts are never removed unless
the caller explicitly passes `--force`; publication is a rename from the
completed staging directory.

## Update boundary

The application manifest pins the feed URL, channel, Ed25519 public key and
integer update policy. The release seed is read only by `publish-update`, must
have owner-only permissions on Unix, and is never available to PHP, Servo or
the frontend.

The updater parses a closed feed schema, reserializes the typed release into its
canonical compact representation and performs strict Ed25519 verification
before selecting an application/channel/platform/architecture artifact. HTTPS
transport is bounded; the downloaded bytes must match both signed length and
SHA-256.

Installation is delegated to a copied helper. The helper waits for the original
process, extracts into a same-filesystem staging directory, verifies every
manifest file, moves the current bundle to one rollback slot, atomically moves
the replacement into place, verifies again and relaunches. The official 1.x
update and release validation covers Linux x86-64 only.

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

Rust extensions remain process-isolated executables. Loading arbitrary dynamic
libraries into the Servo host is intentionally unsupported because it would
expand the memory-safety and ABI boundary.
