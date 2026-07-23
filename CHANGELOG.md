# Changelog

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
