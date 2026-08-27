# Native Workstation

PAM Desktop Workstation is the production profile for IDEs, communication clients, media applications, financial software and internal tools. It combines the native host with a small, dependency-free professional UI runtime. PAM Native is not required.

## Enable the profile

```php
use Pam\Desktop\{Desktop, InstallerScope, PerformanceBudget, ProcessIsolation, ReleaseChannel, RenderBackend, WindowProfile, Workstation};

$desktop->workstation(
    Workstation::defaults()
        ->instance(single: true, forwardArguments: true)
        ->processes(ProcessIsolation::PerWorkspace, poolSize: 4)
        ->agent(background: true, persistentServices: true)
        ->persistence(autosaveMilliseconds: 1_000)
        ->runtime(idleSleepMilliseconds: 20_000)
        ->rendering(RenderBackend::Automatic, dirtyRegions: true)
        ->window('settings', WindowProfile::child('main'))
        ->window('command-palette', WindowProfile::palette('main'))
        ->performance(new PerformanceBudget(frameP95Milliseconds: 16.67))
        ->release(ReleaseChannel::Stable, InstallerScope::CurrentUser)
);
```

Values cross the PHP/Rust boundary as validated integer enums. Unknown window relationships, invalid budgets and unsafe pool sizes fail during boot, before a window is shown.

## Professional UI runtime

The assets are embedded in the executable and add no npm dependency.

```html
<link rel="stylesheet" href="/_pam/workstation.css">
<script defer src="/_pam/bridge.js"></script>
<script defer src="/_pam/workstation.js"></script>
```

### Commands and shortcuts

```js
PamWorkstation.commands.register({
  id: "file.save",
  title: "Save document",
  category: "File",
  shortcuts: ["CommandOrControl+KeyS"],
  enabled: ({ document }) => document?.dirty === true,
  run: ({ document }) => document.save(),
});

const matches = PamWorkstation.commands.search("save");
```

Registration rejects duplicate IDs and conflicting normalized shortcuts. Commands support contextual visibility and enabled state, native menus, contextual menus and command palettes.

### Millions of rows

```js
const list = PamWorkstation.createVirtualList(
  document.querySelector("#files"),
  { count: 2_000_000, estimateSize: 30, overscan: 10 },
  (index) => {
    const row = document.createElement("div");
    row.role = "listitem";
    row.textContent = files[index].name;
    return row;
  },
);
```

Only the visible window plus overscan is mounted. The viewport uses containment, passive scrolling and resize-driven recalculation. The same primitive supports lists, tables and trees.

### Transactional undo and recovery

```js
PamWorkstation.undo.begin("Rename file");
PamWorkstation.undo.record(() => rename(after), () => rename(before));
PamWorkstation.undo.commit();

const journal = new PamWorkstation.RecoveryJournal("editor");
journal.save(documentId, editor.serialize());
const recovery = journal.recover(documentId);

const workspace = new PamWorkstation.WorkspaceStore("main");
workspace.save({ docks, tabs, activeDocument });

// Import every typed PHP/Composer command into the native command palette.
const unregister = PamWorkstation.registerComposerCommands(window.pam);
```

`RenderScheduler.invalidate(region, render)` coalesces invalidations into one animation frame. `PerformanceGate` records bounded samples and reports p50/p95 violations for IPC and rendering.

With `treeShaking` enabled, the generated `window.pam` object exposes only the
native namespaces authorized by the bootstrap contract. For example, an app
without process capability receives neither `process` nor `terminal`; an app
without filesystem roots receives neither `fs` nor background `search`.
Core typed commands and events remain available. Setting `treeShaking: false`
keeps the complete compatibility surface while enforcement still occurs in Rust.
Composer commands are exposed as immutable name/execution metadata only; their
payloads and handlers remain behind the authenticated typed command bridge.

## Capability map

| Workstation concern | PAM Desktop contract |
|---|---|
| Windows and monitors | Multiple native windows, roles/ownership, restore profile, scale-aware geometry and native effects |
| Shell integration | Tray, native menus, local/global shortcuts, badges/progress, notifications, protocol/file activation |
| Data transfer | Typed command bridge, cancellation, backpressure, streamed file reads/writes, rich text/HTML/RGBA clipboard and drag/drop |
| Long-running work | Parallel PHP execution lanes, background jobs, process allowlists, persistent agent profile |
| Integrated terminals | Real cross-platform PTY sessions with ANSI bytes, incremental output, resize, exit status and cancellation |
| Local data | Dedicated SQLite service with WAL, migrations and bounded results; coalesced file watching |
| Extensibility | Lazy PHP/Rust plugins with explicit filesystem, HTTP, process, secret and portal capabilities |
| Reliability | Single-instance profile, autosave journal, workspace restoration, signed updates and rollback policy |
| Performance | Startup snapshot profile, lazy plugins, capability tree-shaking, dirty regions, virtualization and idle sleep |
| Professional UX | Commands, palette foundation, docking/tab state, transactional undo, full keyboard focus and live announcements |
| Operations | Runtime inspector, bounded diagnostics, OTLP counters, visual automation and release performance gates |

In development, `pam.automation` deterministically exercises declared menus,
shortcuts, window focus/visibility/close effects and drag enter/drop/leave. It is
token-authenticated, schema-validated, restricted to project fixtures for drag
data, and absent from production bridge surfaces. This makes shell interactions
testable without coordinate-driven scripts; see [Visual regression testing](visual-testing.md).

Platform-specific signing, installer scope and shell behavior are covered in [Distribution](distribution.md). Security-sensitive integrations are deny-by-default; see [Capabilities](capabilities.md) and [Permission audit](permission-audit.md).

When `agent(background: true, persistentServices: true)` is enabled, PAM starts a
single-instance headless operating-system process with its own supervised PHP
worker, persistent scheduler journal and initialized local services. The graphical
Servo host does not run a duplicate scheduler; closing its last window terminates
the renderer while the agent survives. Reopening the app uses the same durable
SQLite/WAL and journal state. An explicit host quit sends a secure local stop
activation to the agent. Startup is fail-closed: the graphical host waits up to
15 seconds for a private, random, create-once readiness marker published only
after the agent owns its single-instance endpoint, boots PHP, initializes local
services and starts its scheduler. Early exit, malformed readiness and timeout
abort startup instead of silently losing background work. When no separate agent is selected but
`persistentServices` is enabled, the hidden host instead enters operating-system
wait and wakes only for timers, tray activation or IPC.

Persistent jobs use an atomic user-data journal and bounded exponential retry. An entry left in running state by a crash is recovered immediately on the next launch. See [Background jobs](background-jobs.md).

Native host panics produce bounded, private, locally retained reports with a
symbolizable backtrace when `crashReports` is enabled. No application payload or
credential is captured; see [Diagnostics](diagnostics.md).

The startup snapshot is a versioned cache of the validated bootstrap contract,
bound to PAM's protocol and a SHA-256 project fingerprint. It never bypasses the
live PHP handshake: a warm launch compares the live contract to the cached one,
and any code, lockfile, schema or contract change becomes a miss and atomically
publishes a replacement. `pam-desktop doctor` and the development ready event
make hit/miss state observable for performance evidence.

## Accessibility contract

Every interactive control needs a programmatic name, visible focus and keyboard equivalent. Virtualized items expose `aria-posinset` and `aria-setsize`; dialogs can use `FocusManager.trap()`, status changes use `PamWorkstation.announce()`, and the stylesheet honors reduced motion. Shipping gates include keyboard-only navigation and screen-reader smoke tests.

## Release gate

No release is publishable until a clean temporary project is initialized, started with `pam dev`, packaged, installed and launched. The gate removes every generated build directory and installer artifact. Startup, idle CPU, memory, IPC p95 and frame p95 remain separately enforced by the performance evidence pipeline. Hosted Windows and macOS jobs own their native packaging/signing checks; Linux owns its native packaging checks.

The executable release gate is `scripts/clean-room-desktop.sh`. It creates a
fresh `pam init --template desktop` project, replaces the published Composer
dependency with an immutable copy of the candidate checkout, requires `doctor`
and `dev` readiness, builds a directory package, installs it into a disposable
per-user home, launches the installed application, and removes the project plus
all Cargo intermediates on success, failure, or interruption. The Linux release
workflow runs this gate before any publication job can start.
