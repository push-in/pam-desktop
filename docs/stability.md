# Stability and support

Pam Desktop 1.0 defines a stable application contract for Linux x86-64. The
public workflow is `pam desktop`; `pam-desktop` remains the internal native
host discovered and delegated to by PAM.

## Version axes

Three versions evolve independently:

| Contract | Version | Compatibility scope |
| --- | ---: | --- |
| Public application API | `1` | PHP objects and frozen JavaScript bridge |
| Worker protocol | `6` | PAM/PHP JSON-lines boot, requests and responses |
| Rust plugin protocol and SDK | `1` | supervised executable transport and SDK surface |

PHP publishes `Application::API_VERSION` and
`Application::PROTOCOL_VERSION`. Rust publishes `PUBLIC_API_VERSION`,
`PROTOCOL_VERSION` and `PLUGIN_PROTOCOL_VERSION`. Trusted frontend code can
feature-check `window.pam.apiVersion`.

The complete `Pam\Desktop` namespace is stable in 1.x. The frozen JavaScript
surface is:

- `apiVersion`, `windowId`, `invoke`, `emit` and `on`;
- `fs`, `dialog`, `clipboard`, `notification`, `database`, `system`,
  `diagnostics`, `updater` and `plugins`;
- the documented methods, payloads, integer discriminators and error codes
  under those namespaces.

Application-owned command, event, job and plugin identifiers are strings.
Statuses, kinds, categories, themes, policies and other coded variants remain
sequential integer enums beginning at `1`.

## SemVer policy

- Patch releases fix defects and security issues without changing a successful
  public contract.
- Minor releases may add optional methods, enum cases, events or fields when an
  existing 1.x application continues to work unchanged.
- Removing or renaming a public symbol, changing a required parameter or
  return type, reassigning an integer discriminator, or changing an existing
  successful behavior requires public API version `2` and a SemVer major.
- Deprecations are documented in a minor release and remain functional for the
  rest of 1.x. They are removed only in a major release.
- Worker protocol `6` remains accepted throughout 1.x. A mandatory transport
  change requires a protocol bump and an explicit compatibility path.

An additive change still updates the compatibility snapshots intentionally so
reviewers see the exact new surface.

## Executable compatibility gates

CI enforces four independent contracts:

1. `compat/php-api-v1.txt` records every public PHP symbol, signature,
   constant, enum case and promoted property through reflection.
2. `compat/protocol-v6/*.json` round-trips representative boot, request,
   response and plugin messages byte-semantically through typed validation.
3. `compat/rust-plugin-v1` compiles as an external consumer of the stable Rust
   SDK.
4. The Linux host archive is produced twice, compared byte-for-byte, verified
   against its schema-1 manifest, installed into clean XDG directories,
   executed and uninstalled.

## Supported platform

The 1.x release and compatibility guarantee is:

- Linux on x86-64;
- glibc compatibility based on Ubuntu 22.04 release builds;
- PHP 8.5 for application workers;
- Rust 1.88 for rebuilding the host or Rust plugins;
- X11 or Wayland environments supported by the pinned Winit/Servo stack.

Tray visibility and global shortcut registration depend on the desktop
environment and compositor. Their documented graceful fallback is part of the
contract.

Windows and macOS have native-host preview support. A dedicated pre-release
matrix runs the portable gateway/plugin tests and Clippy contracts, builds the
real Servo host natively on macOS arm64 and Windows x64, executes its bounded
version handshake, and creates target-labelled host archives with adjacent
SHA-256 files and a schema-1 per-file integrity manifest. A separate compact
14-day evidence artifact binds the source commit, toolchain, binary hash and
byte size. Build intermediates are always removed. Tagged releases cannot
publish through either the native-host or API-only path unless that platform
matrix and the complete source CI both pass. Native host archives are therefore
installable by the Composer bootstrap on all three platforms. This remains a
preview—not the Linux 1.x compatibility guarantee—until graphical clean-room,
installer, certificate signing/notarization and updater evidence also pass on
the corresponding operating system.

## Trust boundary

Local frontend assets and registered PHP plugins are trusted application code.
Rust plugins are process-isolated for lifecycle, crash and ABI safety. Their
compatible default inherits user authority; Linux applications can select the
fail-closed strict namespace sandbox and explicit per-plugin capabilities.
Strict mode is rejected on platforms that have not yet earned sandbox evidence.
Remote arbitrary pages, dynamic libraries in the host and generic shell or
filesystem bridges are outside the public contract.

Servo rendering internals may evolve with the pinned engine. PAM guarantees
the documented application, transport, lifecycle and packaging behavior, not
feature-for-feature parity with another desktop framework.
