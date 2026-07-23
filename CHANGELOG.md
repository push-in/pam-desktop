# Changelog

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
