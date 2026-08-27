# Native Workstation certification

This is the release evidence map for the 50 Native Workstation contracts. A
row is not certified by documentation alone: its source contract, automated
test or native CI gate must remain present. Platform features fail explicitly
when the operating system or an application capability does not provide them;
PAM never reports a successful no-op.

| # | Production contract | Evidence and gate |
|---:|---|---|
| 1 | Configurable worker isolation per app, window or workspace | `ProcessIsolation`, keyed `WorkerPool`, protocol/PHP tests |
| 2 | Child, modal, popover, panel and palette window roles | `WindowRole`, ownership validation, Servo window registry |
| 3 | Geometry, monitor and workspace restoration | private atomic `WindowStateStore`, window-state tests |
| 4 | Mixed monitor scale and density | stable monitor identity, logical/physical conversion and Winit scale events |
| 5 | Tray, Dock/taskbar badge and progress, launcher quick actions | native shell tests; Linux Desktop Actions; Windows Jump List; macOS Dock status |
| 6 | Dynamic native/context menus | typed menu tree and enable/check effects |
| 7 | Local and global shortcuts with conflict rejection | normalized accelerator registry and native registration |
| 8 | Cross-window/system drag and drop | typed enter/hover/drop/leave events and capability-scoped file grants |
| 9 | Rich clipboard | text, HTML, RGBA image, capability-granted files and bounded custom OS formats |
| 10 | Open/folder/save dialogs with persistent grants | dialog gateway plus private grant journal |
| 11 | Protocol and file associations | packaged metadata and validated activation arguments |
| 12 | Secure single instance and argument forwarding | private local IPC, bounded envelope and acknowledgement tests |
| 13 | Separate background agent | headless process, single instance and post-bootstrap readiness handshake |
| 14 | Services without a visible window | agent-owned scheduler, SQLite, search, process and filesystem services |
| 15 | Persistent jobs and crash recovery | atomic job journal, overlap/retry/recovery tests |
| 16 | Typed binary IPC, backpressure and cancellation | protocol 6, bounded streams, cancellation tokens and golden fixtures |
| 17 | Constant-memory large-file streaming | chunked read/write endpoints, byte limits and stream cancellation |
| 18 | PHP process pools | bounded supervised pools, round-robin and affinity routing |
| 19 | Configurable command/plugin sandbox | strict Linux bubblewrap namespaces, inherited compatibility mode, fail-closed unsupported-platform behavior |
| 20 | Per-plugin filesystem/network/shell/device permissions | typed integer policy, explicit mount/network/shell/device exposure and critical permission-audit finding for inherited authority |
| 21 | Real PTY | `portable-pty`, resize, ANSI bytes, signals, multiple session registry |
| 22 | Incremental process manager | bounded stdout/stderr chunks, exit events, cancellation and timeout |
| 23 | SQLite worker with WAL and migrations | dedicated native database service and migration journal |
| 24 | Coalesced filesystem watching | capability-scoped watcher, debounce and bounded event batches |
| 25 | Background file index/full-text search | persistent index, incremental watcher updates and bounded query results |
| 26 | Signed incremental update and rollback | Ed25519 feeds, staged verification, atomic swap and previous-version recovery |
| 27 | Stable, beta and nightly channels | sequential `ReleaseChannel` contract and channel-bound feeds |
| 28 | Per-user or per-machine install | `InstallerScope`, rootless and native installer layouts |
| 29 | MSI, DMG/PKG, AppImage, DEB and RPM | native packager format gates and host-specific tools |
| 30 | Windows signing, macOS notarization, Linux metadata | Authenticode/timestamp, codesign/notarytool/staple and Freedesktop metadata gates |
| 31 | Deep sleep while idle | lifecycle idle deadline and event-driven wake-up |
| 32 | Immediate startup snapshot | project fingerprint, protocol/schema validation and warm/cold budget evidence |
| 33 | Lazy plugin/module initialization | lazy supervisors and on-demand native namespaces |
| 34 | Capability tree shaking | bootstrap-derived frozen bridge surface tests |
| 35 | GPU acceleration with safe fallback | typed render backend and automatic/software fallback diagnostics |
| 36 | Dirty-region partial rendering | frame-coalesced keyed invalidation scheduler |
| 37 | Virtualized lists/tables/trees | bounded DOM window, overscan and ARIA set metadata |
| 38 | IDE docking layout | serializable panel order/direction/size model |
| 39 | Detachable tabs | tab transfer event bound to declared window identities |
| 40 | Composer-extensible command palette | immutable Composer command metadata and typed invocation |
| 41 | Transactional undo/redo | bounded grouped operations, rollback and state events |
| 42 | Autosave, recovery journal and crash restore | bounded journals and restored workspace/window state |
| 43 | Native notifications with actions/replies | bounded action IDs and typed native response events |
| 44 | Camera, microphone, printing and scanner integration | consent-gated desktop portal, PDF print and capability-scoped scanner output |
| 45 | Keyboard/screen-reader accessibility | focus traps, names, live regions, reduced motion and virtual-set metadata |
| 46 | Deterministic shell automation | development-only authenticated menu/window/shortcut/drag endpoint |
| 47 | IPC/process/memory/frame/plugin DevTools | authenticated runtime inspector and bounded diagnostics snapshot |
| 48 | Symbolizable crash reports and reproducible sessions | private bounded crash reports plus diagnostic/performance evidence manifests |
| 49 | Startup/idle/RAM/IPC/render performance gates | cold/warm budgets, percentile evidence and footprint regression gate |
| 50 | Clean-room create/dev/package/install/launch | `scripts/clean-room-desktop.sh` and release job, with unconditional cleanup |

## Platform acceptance

- Linux x86-64 is the stable 1.x target and owns the complete graphical
  clean-room, deterministic archive, install/uninstall and footprint gates.
- macOS arm64 and Windows x64 own native compilation, binary execution,
  target-labelled deterministic host archive verification and artifact
  attestation. They remain preview targets until native graphical installer and
  signing clean-room evidence is available from configured certificate secrets.
- Signing is never weakened when secrets are absent. `--sign` fails closed;
  unsigned CI cannot be presented as notarized or Authenticode-certified output.

Every build job finishes with `cargo clean`, and the clean-room trap removes its
temporary application, installed copy, package output and build intermediates
on success, failure or interruption.
