# Changelog

## Unreleased

- Add explicit-opt-in, privacy-bounded OTLP HTTP/JSON spans for Desktop command
  execution with a non-blocking bounded exporter, diagnostic counters and
  official Collector interoperability certification.
- Continue validated W3C version `00` trace lineage from authenticated bridge
  invocations into distinct Desktop command child spans.

- Add a bounded, authenticated `pam desktop diagnostics` transport for live
  development snapshots, backed by an ephemeral private session descriptor.
- Add authenticated Desktop package-footprint evidence for compressed,
  installed and executable bytes, plus a release comparison gate that rejects
  regressions beyond the accepted baseline.

## 1.2.1

- Align the Rust workspace, packaged host and Composer launcher on version
  1.2.1 after the additive 1.2.0 PHP API publication.

- Add the convention-first `App` authoring layer aligned with PAM Native:
  `#[Desktop]`, typed `#[Command]` methods and invokable classes, constructor
  injection, DTO hydration, integer-backed enum binding and ordinary PHP
  return normalization.
- Add invocation-scoped `WindowHandle`, `Windows`, typed `DesktopWindow`,
  `Events` and `ApplicationControl` services that collect native effects and
  event objects without exposing host handles or protocol envelopes.
- Add declarative `#[Window]`, `#[Menu]`, `#[MenuItem]`, `#[MenuSeparator]` and
  `#[Listen]` discovery while preserving the immutable API 1 builders as the
  advanced layer.
- Add focused authoring, typed-command, window and menu documentation plus a
  convention-first README quick start.

- Add capability-scoped, backpressured binary read/write streams without
  base64 payload expansion.
- Add capability-scoped native HTTPS with named origins, path confinement,
  bounded bodies, strict headers and integer method contracts.
- Harden Rust sidecars with optional SHA-256 pinning on every start and a clean
  inherited environment.
- Add Linux single-instance activation, deep-link/file forwarding, MIME and URL
  associations, and opt-in XDG autostart packaging.
- Add capability-gated Linux Secret Service storage with encrypted D-Bus
  sessions, application scoping and bounded UTF-8 values.
- Add manifest-allowlisted bundled process execution with integer argument
  policy, empty environments, bounded concurrent I/O and hard timeouts.
- Add capability-scoped public file watching and direct XDG portal integration
  for URI opening, interactive screenshots and native PDF printing.
- Add a development-only responsive runtime Inspector with accessible live
  aggregate metrics and no sensitive payload collection.

- Added capability-scoped native SQLite with a bundled engine, prepared
  parameters, WAL, foreign keys, bounded queries and atomic transactions.
- Added explicit stateful, parallel and background PHP command execution lanes
  backed by a lazy supervised worker pool.
- Added privacy-bounded Linux system snapshots for CPU, memory, uptime,
  connectivity, power and battery state.
- Added decorated, transparent, always-on-top, maximized and fullscreen window
  policy plus typed runtime effects for fullscreen, maximization and stacking.
- Added authenticated runtime diagnostics and a reproducible bridge benchmark
  harness.
- Expanded architecture, capability, database, execution, diagnostics, window,
  system-information and performance documentation.

## 1.1.2

- Added GitHub artifact provenance for the byte-reproducible Linux x86-64 host
  archive.
- Made release-workflow changes participate in native-host change detection so
  supply-chain changes cannot produce an API-only release without assets.
- Updated the public version record and completed the missing 1.0.5–1.1.1
  changelog history.

## 1.1.1

- Preserved secondary Servo WebViews and rendering contexts when their native
  close control is used, preventing shared EGL display invalidation on
  GLVND/NVIDIA systems.

## 1.1.0

- Added the concise `Application::make(...)`, `Window::load(...)`, and
  `Window::defaults(...)` DSL while preserving the complete stable API 1
  compatibility surface.

## 1.0.5

- Released old Servo window generations before hot-reload replacements are
  created so an old EGL connection cannot invalidate a new shared display.

## 1.0.4

- Forced bindgen and mozangle to use the Clang 18 executable and resource
  headers together on the GLIBC 2.35 build runner.

## 1.0.3

- Restored the GLIBC 2.35 build baseline for compatibility with Ubuntu 22.04,
  Pop!_OS 22.04 and newer Linux distributions.
- Kept Clang 18 for the Servo build without raising the runtime GLIBC baseline.

## 1.0.2

- Added a Composer vendor binary that downloads the matching native desktop host
  on first use, verifies its SHA-256 checksum and reuses the cached installation.
- Removed the need for a separate manual `pam-desktop` installation when a project
  is created with Composer.
- Updated the production Linux build to Clang 18 on Ubuntu 24.04.

## 1.0.1

- Published the PHP application API as `pushinbr/pam-desktop` from the
  repository root so public `pam init --template desktop` projects resolve
  without custom Composer repositories.
- Updated packaged vendor-path handling and compatibility fixtures for the
  organization-owned Composer coordinate.

## 1.0.0

- Declared public API version `1` while retaining worker protocol `6` and Rust
  plugin protocol `1` for the complete 1.x line.
- Added an exact reflection snapshot for every public PHP symbol, method
  signature, constant, enum case and promoted property.
- Added golden protocol-6 bootstrap, request, response and plugin transport
  fixtures with deserialize/validate/reserialize compatibility tests.
- Added a standalone Rust plugin consumer that compiles the complete stable SDK
  surface as part of the locked workspace.
- Added `window.pam.apiVersion`, the Rust `PUBLIC_API_VERSION` constant and
  explicit SemVer, deprecation and support policies.
- Tightened the stable Linux shell contract to one tray-backed menu and made
  invalid shell effects fail explicitly.
- Excluded standard Rust plugin scaffold sources from application bundles while
  retaining and hashing configured release executables.
- Documented that process-isolated Rust plugins are trusted native code, not an
  operating-system sandbox.
- Added deterministic Linux x86-64 host archives with a versioned manifest,
  per-file sizes and SHA-256 digests, normalized metadata and an adjacent
  archive checksum.
- Added atomic rootless XDG install/uninstall scripts that refuse unrelated
  command paths and affect only the exact installed version.
- Added end-to-end host archive validation for safe members, exact file sets,
  checksums, byte-for-byte reproducibility, installed execution and uninstall.
- Prevented a custom application output directory from entering its own bundle
  and added a byte-for-byte portable archive reproducibility regression test.
- Focused the official 1.x build, release, documentation and compatibility
  guarantee on Linux x86-64 built from Ubuntu 22.04.

## 0.6.0

- Added protocol 6 contracts for native shell configuration, supervised jobs,
  PHP plugins, Rust plugins and dynamic shell effects.
- Added immutable PHP `Shell`, `Menu`, `MenuItem`, `Tray`, `GlobalShortcut`,
  `BackgroundJob` and `RustPlugin` configuration APIs with sequential
  integer-backed variants.
- Added composable PHP plugins that register only through the public
  `Application` API and are reported in the validated boot contract.
- Added the `pam-desktop-plugin` Rust SDK with a bounded, versioned JSON-lines
  process protocol and typed metadata, results, failures and events.
- Added a Rust plugin supervisor with exact export enforcement, per-plugin
  serialization, deadlines, cancellation and crash recovery without replay.
- Added `pam desktop plugin new` and `pam desktop plugin build` scaffolding for
  project-local, release-built native extensions.
- Added native menu trees, checkbox state, status tray activation and
  close-to-tray behavior; Linux uses the D-Bus StatusNotifierItem protocol
  without a GTK/AppIndicator runtime dependency.
- Added global shortcuts with graceful registration fallback and typed
  press/release lifecycle events.
- Added periodic PHP background jobs with interruptible shutdown, initial
  delay, timeout, skip/wait overlap policy, typed effects and ordered lifecycle
  events.
- Added hot-reload replacement of the complete plugin, scheduler and native
  shell configuration without leaving duplicate timers or registrations.
- Focused tagged host artifacts and official 0.6 package validation on Linux
  x86-64 while preserving the existing non-Linux packager code for future work.

## 0.5.0

- Added protocol 5 update configuration with integer-backed policy, platform,
  artifact-kind and lifecycle-state enums.
- Added immutable PHP `Updates` configuration with pinned HTTPS feed, channel,
  Ed25519 public key and manual, notify or automatic policy.
- Added the frozen `pam.updater` API and background feed checks/downloads.
- Added strict typed feed parsing, canonical Ed25519 verification, bounded
  HTTPS transport, signed byte lengths and SHA-256 artifact verification.
- Added detached update application with parent-process coordination, complete
  bundle verification, atomic directory swap, rollback and relaunch.
- Added owner-only update-key generation and deterministic multi-platform feed
  publication without exposing the private seed.
- Added a native launcher for Windows and macOS bundles.
- Added cross-platform portable ZIP packaging, macOS `.app`/DMG metadata,
  Windows MSIX metadata and platform icon generation.
- Added hardened-runtime codesigning, optional Apple notarization/stapling and
  certificate-store Authenticode signing.
- Added Linux, macOS and Windows CI contracts and tagged host release builds.

## 0.4.0

- Added protocol 4 application manifests with validated reverse-DNS identity,
  portable versions, publisher, description, icon and bundle exclusions.
- Added sequential integer-backed `ApplicationCategory` contracts in PHP and
  Rust with Freedesktop and Debian mappings.
- Added safe PNG/SVG icon validation and generated application metadata.
- Added `pam desktop run` production mode without the development watcher.
- Added atomic `pam desktop build` staging with exact-artifact replacement only
  behind `--force`.
- Added self-contained Linux directories containing the application, Pam
  worker, Servo host, isolated `php.ini` and non-glibc runtime libraries.
- Added materialization of Composer path packages, secret/build exclusions and
  explicit rejection of unsafe non-vendor symlink escapes.
- Added deterministic portable `.tar.gz` archives with per-user install and
  uninstall scripts.
- Added optional native Debian packages with `/opt`, `/usr/bin`, Freedesktop
  desktop entry and hicolor icon integration.
- Added a sorted bundle manifest containing application/runtime/target metadata
  and SHA-256 integrity records for every shipped file.
- Updated the Pam starter with a typed 0.4 manifest, custom vector icon and
  `desktop:build` Composer command.

## 0.3.0

- Added PHP-declared native capabilities through immutable `Capabilities` and
  named `FileSystemRoot` contracts.
- Added capability-based filesystem access with read, write and read-write
  permissions, relative paths, UTF-8 limits and symlink/traversal rejection.
- Added process-lifetime opaque grants for files and directories selected by
  dialogs or dropped onto a window; frontend code never receives ambient paths.
- Added native open, multi-open, save and directory dialogs with validated
  filters and main-event-loop ownership.
- Added separately gated clipboard read/write/clear operations and persistent
  host ownership of clipboard content.
- Added native notifications with bounded title/body content and sequential
  integer urgency values.
- Added Winit drag enter, leave, drop and error events targeted to the receiving
  application window.
- Added the frozen `pam.fs`, `pam.dialog`, `pam.clipboard` and
  `pam.notification` frontend APIs.
- Added protocol 3 contracts, capability error codes and security tests for
  traversal, permissions and symbolic links.
- Updated the Pam Desktop starter with a PHP-authorized Native Lab and protocol
  3 diagnostics.

## 0.2.0

- Added bidirectional application events with `window.pam.emit`, `window.pam.on`,
  `Application::on` and `ClientEvent`.
- Added per-request deadlines, `AbortSignal` cancellation and explicit timeout,
  cancellation and crash error codes.
- Added supervised worker recovery without automatic command replay.
- Added multiple Winit/Servo windows, per-window entries and targeted title,
  visibility, close and focus effects.
- Added development hot reload with asset refresh, PHP worker restart,
  bootstrap revalidation and visible reload-error events.
- Added the protocol 2 contract, bounded event history, secure window routes and
  worker recovery tests.
- Added PHPStan level 9 validation for the public PHP package.
- Unified the public workflow under `pam desktop`; `pam-desktop` remains the
  separately shipped native host.
