# Changelog

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
