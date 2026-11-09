# Architecture

PAM Desktop is two processes and a contract. A PHP worker describes the interface; a Rust host lays it out, keeps a retained scene and drives the views of the platform through one trait, `Backend`. This page explains how the pieces fit, for people who want to debug an application, port a backend or change the host.

```text
┌──────────────────────┐   stdin: events, module results   ┌───────────────────────────────────────────┐
│ PHP worker process   │ ◀──────────────────────────────── │ pam-desktop host process                  │
│                      │                                   │                                           │
│ components, state    │   stdout: frames, module calls    │ engine ─▶ render core ─▶ Backend          │
│ render() ─▶ encoder  │ ────────────────────────────────▶ │ (layout,   (scene,       (GTK 4 / AppKit  │
└──────────────────────┘                                   │  diff)      controllers)  / Win32+D2D)    │
                                                           └───────────────────────────────────────────┘
```

## Processes

The host is the `pam-desktop` binary (`crates/pam-desktop-host`). It opens the first window, then starts the worker:

```text
<php> packages/desktop/bin/pam-desktop-worker <entry script>
```

The worker bootstrap opens the channel, runs the entry script (which calls `App::run()` and produces the first frame), reports that it is ready and then loops: it blocks reading its standard input and handles one batch of host messages per turn. While the interface is idle PHP is asleep in that read.

- The worker's **stdout** and **stdin** are the channel. Output from `echo` and `print` is captured and redirected to stderr.
- The worker's **stderr** is inherited from the host: PHP warnings and the application's own diagnostics appear where `pam-desktop` was started.
- The worker runs in the project directory with the environment described in [Getting started](getting-started.md#environment-variables).

When the worker exits, a production host quits; a development host keeps the window on its last frame and starts a new worker at the next source change. Closing the main window tells the worker to shut down.

## The channel

Both directions carry the same framing:

```text
u32 length (little-endian) · u8 type · payload
```

`length` counts the type byte and the payload. A message longer than 64 MiB is malformed and ends the channel.

Worker to host:

| Type | Name | Payload |
| --- | --- | --- |
| `0x01` | Commit | A tree frame (`PNT1`) or a patch frame (`PNP1`). |
| `0x02` | Call | `u64 request · u16 module length · u16 method length · module · method · payload`. A module call by name. |
| `0x03` | Call typed | `u64 request · u16 operation · payload`. A module call by operation number. |
| `0x04` | Error | An error report: `PAMERR1`, a line break and a JSON object, or plain text. |
| `0x05` | Ready | The entry script finished booting. |
| `0x06` | Idle | Everything up to the last flush marker was handled. |

Host to worker:

| Type | Name | Payload |
| --- | --- | --- |
| `0x01` | Event | `u64 node · u16 event kind · payload`. An element event, or an application event with node 0. |
| `0x02` | Module result | `u64 request · u8 status · payload`. Status 1 is success, 2 is failure. |
| `0x03` | Flush | Ends a batch. |
| `0x04` | Resync | The host rejected a frame: send a complete tree. |
| `0x05` | Shutdown | Leave the loop and exit. |
| `0x06` | Checkpoint | Save the hot reload checkpoint. |

### A turn

The host does not send an event and wait. It queues what happened during one drain of its own loop (several events, several module results), writes them, and ends with one flush marker. The worker dispatches every message of the batch, renders **once** for all of them, commits a frame if anything changed and answers Idle.

Commits are applied asynchronously. The worker does not wait for the host to accept a frame; a frame the host cannot apply comes back as a Resync.

### Module calls

A module call carries a request id. The host answers each request exactly once, at any later time and from any thread: when a timer fires, when a download finishes. A request made by a worker that was reloaded away is answered to nobody, because results are tagged with the generation of the worker that asked.

The payloads of events and module calls are `WireMap`s, a bounded key/value envelope:

```text
u16 count · ( u16 key length · key · u8 tag · value )*
tag 1 = string (u32 length + UTF-8)   tag 2 = i64   tag 3 = f64
tag 4 = bool (u8)                     tag 5 = opaque bytes (u32 length)
```

A failure payload is not a map: it is the message, as UTF-8 text.

## Frames

`crates/pam-desktop-wire` defines three binary formats, all at protocol version 1.

| Magic | Name | Direction | Content |
| --- | --- | --- | --- |
| `PNT1` | Tree frame | PHP to engine | A complete tree: every node with its kind, key, properties and children. |
| `PNP1` | Patch frame | PHP to engine | The difference from the previous frame, as PHP computed it. |
| `PNB1` | Mutation batch | Engine to core | The operations to apply to the scene. |

A node has a kind (`Column`, `Text`, `Input`, `Scroll`, `Modal`, `NavigationHost`, `VirtualList`, `Media`, `WebView`…), a 64-bit id and a bag of properties keyed by a 16-bit identifier. Event handlers are properties too: the presence of `OnPress` tells the host that PHP wants presses of that node.

A mutation batch holds six operations:

| Mutation | Meaning |
| --- | --- |
| `Create(node)` | A node was mounted. |
| `Remove { id }` | A node was unmounted. |
| `Update { id, key, value }` | One property changed (or was unset). |
| `Move { id, parent, index }` | A node changed place. |
| `Layout { id, frame }` | The engine computed a new frame for a node. |
| `SetRoot { id }` | The root of the tree. |

The wire format has limits: a frame of at most 16 MiB, 100,000 nodes, a depth of 512, 128 properties per node and 1 MiB per value.

### Property and event tables

Property and event identifiers are shared with PAM Native so that the two projects speak the same protocol:

| Range | Table |
| --- | --- |
| Properties 1–543, events 1–80 | The core table. It is closed. |
| From 1001 | The extension table: identifiers added after the core table closed. |
| From 2001 | The desktop table: identifiers that exist only in PAM Desktop. |

The desktop table:

| Id | Property | | Id | Property |
| --- | --- | --- | --- | --- |
| 2001 | `Cursor` | | 2015 | `WindowMinWidth` |
| 2002 | `Tooltip` | | 2016 | `WindowMinHeight` |
| 2003 | `WindowTitle` | | 2017 | `CaptureKeys` |
| 2004 | `WindowDrag` | | 2018 | `DragType` |
| 2005 | `OnHoverIn` | | 2019 | `DropType` |
| 2006 | `OnHoverOut` | | 2020 | `OnDragEnter` |
| 2007 | `OnKeyDown` | | 2021 | `OnDragLeave` |
| 2008 | `OnKeyUp` | | 2022 | `GridTracks` |
| 2009 | `OnFilesDrop` | | 2023 | `InputFormatDecimalMinDigits` |
| 2010 | `WindowSurface` | | 2024 | `ScrollbarGutter` |
| 2011 | `WindowWidth` | | 2025 | `ScrollbarWidth` |
| 2012 | `WindowHeight` | | 2026 | `ScrollbarThickness` |
| 2013 | `WindowResizable` | | 2027 | `ScrollbarColor` |
| 2014 | `WindowModal` | | 2028 | `TextSpanBoxes` |
| 2029 | `TextInlineBlock` | | 2037 | `InlineLevel` |
| 2030 | `TableDisplay` | | 2038 | `ScrollBothAxes` |
| 2031 | `TableLayout` | | 2039 | `CanvasBitmap` |
| 2032 | `BorderCollapse` | | 2040 | `ScrollbarTrackColor` |
| 2033 | `VerticalAlign` | | 2041 | `InputSize` |
| 2034 | `LengthUnits` | | 2042 | `MenuListOptions` |
| 2035 | `CompositedLayer` | | 2043 | `MinWidthAuto` |
| 2036 | `FontSmoothing` | | | |

| Id | Event |
| --- | --- |
| 2001 | `HoverIn` |
| 2002 | `HoverOut` |
| 2003 | `KeyDown` |
| 2004 | `KeyUp` |
| 2005 | `FilesDrop` |
| 2006 | `DragEnter` |
| 2007 | `DragLeave` |

Before starting the worker the host exports the highest identifiers it implements in `PAM_RENDERER_CONTRACT`. The PHP package never sends an extension identifier that the host did not declare; [Compatibility](compatibility.md) says what happens instead and which feature needs which identifier.

## The crates

| Crate | Role |
| --- | --- |
| `pam-desktop-wire` | Encoding and decoding of frames, patches and mutation batches; the property, event and node kind tables; the protocol handshake. |
| `pam-desktop-engine` | Keeps the last tree, applies patches, runs layout (flexbox, grids, text measured through a callback) and emits mutation batches. |
| `pam-desktop-ui` | The platform-neutral render core: the worker channel, the scene, one controller per feature, the modules that need only the standard library, and the `Backend` trait. |
| `pam-desktop-gtk`, `pam-desktop-appkit`, `pam-desktop-win32` | One implementation of `Backend` each. |
| `pam-desktop-host` | The binary: command line, `pam-desktop.json`, `init`, application directories, the HTTP client with a TLS stack, SQLite, the keychain, packaging and updates. It starts the backend of the system it was built for. |

The wire and engine crates started as a copy of PAM Native's and are licensed under Apache 2.0; the rest is MIT. See `NOTICE`.

## The render core

Everything in the core runs on the UI thread. `runtime.rs` is the glue: one pass of `drain` applies all the work that is queued, then flushes the scene to the backend and the outgoing events to the worker, once.

```text
backend events ─┐
                ├─▶ drain ─▶ controllers settle ─▶ scene.flush(backend) ─▶ worker.flush()
inbox (threads) ┘
```

1. **Backend events** (pointer, key, focus, input edits, scroll, frame ticks, window changes) are pushed by native callbacks on the UI thread.
2. **Inbox** items come from other threads: worker messages, module results, a reload request from the file watcher, image and media downloads.
3. A committed frame goes to the engine, which answers with a mutation batch. The scene applies it and records what changed.
4. Every **controller** sees each change and reacts. Controllers may change the scene themselves (add chrome, set overrides), so the runtime repeats until the scene is quiet.
5. The **scene** pushes the minimal set of commands to the backend: frames, decorations, text, layers, children.
6. Events produced for PHP during the pass are sent, followed by one flush marker.
7. A backend may report events from inside a `Backend` call made during the pass (a scroll that lands, a focus that moves). `drain` then runs another pass for them, up to eight passes in a row.

### The scene

`scene.rs` holds every node the engine mounted, plus the nodes the core creates for its own chrome (tab bars, modal backdrops, headers, toasts, error reports). Core-made nodes have ids with the top bit set, so they never collide with PHP ids.

For each node the scene resolves ready-to-paint state from the properties: a `Decoration` (background, borders, radii, shadow), a `LayerState` (opacity, transform, clipping), text and image content. Three mechanisms let controllers change how a node looks without touching its properties:

- **Layer overrides** multiply opacity and add a transform. Presentation (navigation, tabs, modals), press feedback and motion each own a slot.
- **Style overlays** are properties layered over the node's own: state styles and the default look of buttons.
- **Presentation values** are what a running animation writes: opacity, translation, scale and rotation.

### Controllers

Each feature is one module with the same shape: `on_change` for scene changes, `after_commit` once a batch is applied, and handlers for the events it cares about. A controller never calls a backend. It changes the scene, queues events for PHP, or queues a host command.

| Module | Owns |
| --- | --- |
| `interaction.rs` | Hit testing; the press state machine (press, long press, double tap, retention); hover; state styles; focus and key events. |
| `inputs.rs`, `input_format.rs` | The controlled-value protocol of text inputs; masks and currency. |
| `motion.rs`, `animator.rs` | Implicit transitions, presets, keyframes, programs, shimmer, on one frame-clock sampler. |
| `scrolling.rs` | Scroll events, programmatic scrolling, sticky headers, list windowing, sections, refresh. |
| `gestures.rs` | Gesture detectors, drag contracts, native transforms. |
| `images.rs` | Source resolution, downloads, the disk cache, shared decoded images, load events. |
| `media.rs` | Source resolution, the media cache, the play gate, progress, clock and bar bindings. |
| `webview.rs` | Page source, navigation policy, load timeout, bridge events. |
| `drawing.rs` | `Canvas` and `DrawingCanvas` as lists of draw commands. |
| `selection.rs`, `dragdrop.rs` | Text selection with the pointer; dragging text between elements. |
| `controls.rs` | The switch and the activity indicator, drawn by the host. |
| `presenter/navigation.rs`, `header.rs`, `tabs.rs`, `modal.rs`, `coverage.rs` | Stack transitions, the header, the tab bar, modals and sheets, route covered and focused events. |
| `windows.rs` | Secondary windows. |
| `accessibility.rs` | Role, name, state and actions of a node. |
| `observers.rs` | Intersect, resize and mutate observers. |
| `error_overlay.rs`, `toast.rs`, `chrome.rs` | Host chrome: error reports and toasts. |
| `runtime_shell.rs`, `shell.rs` | The `desktop` module: menus, tray, shortcuts, dialogs, notifications, window commands. |
| `modules/*`, `runtime_modules.rs` | Storage, files, HTTP, image prefetch, timers, system information, device status (`probe.rs`), links; calls that need the window or the backend. |
| `watch.rs` | The development file watcher. |

Navigation, tabs and modals are implemented here, on top of plain scene nodes, instead of with each platform's navigation controllers and dialogs. That is why they behave identically on every backend.

## The `Backend` contract

`crates/pam-desktop-ui/src/backend.rs` is the trait every platform implements. The core owns the scene; a backend owns the native objects. **Backends never interpret wire properties.** They receive resolved state through small retained-mode commands and report raw input back.

A node is backed by one of seven view classes:

| `ViewClass` | Native object |
| --- | --- |
| `Box` | A decorated box with optional text and image content, and children. |
| `Scroll` | A scrolling viewport; its children live in content coordinates. |
| `Input` | A native single-line text field. |
| `TextArea` | A native multi-line editor. |
| `Surface` | Pixels painted from draw commands (canvas, drawing surface). |
| `Media` | A video or audio player. |
| `WebView` | An embedded web page. |

The commands, by group:

| Group | Methods |
| --- | --- |
| Windows | `open_window`, `close_window`, `window_metrics`, `set_window_root`, `set_window_overlays` |
| Views | `create_view`, `destroy_view`, `set_children`, `set_frame`, `set_decoration`, `set_layer`, `set_text`, `set_focusable`, `set_accessibility`, `focus` |
| Text input | `set_input_config`, `set_input_text`, `set_input_selection` |
| Images and content | `load_image`, `release_image`, `set_image`, `text_index_at`, `set_text_selection`, `begin_text_drag`, `set_media`, `media_position`, `set_web_view`, `set_drawing` |
| Scrolling | `set_scroll_content`, `set_scroll_config`, `scroll_to` |
| Text | `measure_text` |
| Frames | `request_frame`, `schedule_wake` |
| System services | `clipboard_set_text`, `clipboard_text`, `show_alert`, `set_appearance`, `announce`, `screen_reader_active`, `set_content_protection` |
| Desktop shell | `set_menu_bar`, `popup_menu`, `set_tray`, `pick_files`, `notify`, `window_command`, `window_state`, `set_shortcuts`, `set_cursor`, `set_tooltip`, `begin_window_drag` |
| Application | `quit` |

`announce`, `screen_reader_active` and `set_content_protection` have default implementations that answer `false`: a backend that cannot do one of them leaves it out.

What a backend reports is one enum, `BackendEvent`: pointer and key events, focus changes, input edits, scroll offsets, wheel, magnify and rotate gestures, dropped files and text, window state and size, appearance changes (`AppearanceChanged`), text scale and reduced motion changes (`MetricsChanged`, after which the core reads `window_metrics` again), links handed to the running application (`OpenUrl`), menu, shortcut, tray and notification activations, media and web view events, decoded images, accessibility actions, and the two clock events `Frame` and `Wake`.

Three properties of the contract are worth knowing:

- **Text is measured by the backend.** The engine asks for the size of a paragraph through `measure_text`, with the same description `set_text` later draws, so a box always fits exactly the text the platform paints.
- **State is declarative where a platform object has a life of its own.** `set_media` and `set_web_view` take the whole state the player or the page should be in; the backend compares it with the previous one and acts on the difference.
- **Interaction is not the backend's job.** A backend forwards raw pointer and key events of a window. Hit testing, presses, hover, gestures, selection and drag thresholds are decided by the core, once, for every platform.

`testing.rs` provides a backend that records every command. The core's tests run against it, without a display.

## What runs without PHP

PHP is never in the path of a frame. The host handles these by itself and tells PHP only the outcome:

| Feature | In the host | PHP hears |
| --- | --- | --- |
| Animations | Implicit transitions, presets, programs and shimmer are sampled on the frame clock and written to the scene as presentation values. | Completion only. |
| Scrolling | The viewport is the native scroll view. List windowing, sticky headers and programmatic scrolls are computed by the core. | Scroll and end-reached events; requests for more rows. |
| Press feedback | Dim, scale and fill animations, hover and state styles. | The press. |
| Gestures | Recognition, drag targets that follow the pointer, driven properties, snap animations, pan and zoom transforms. | Begin, end, cancel, settle; coalesced updates if asked. |
| Media | Play and pause from intent and visibility, visible autoplay, clock labels and progress bars bound to a player. | Ready, end, error; progress four times a second if asked. |
| Text input | Typing, selection, input methods, masks. | Edits, according to the input's sync mode. |
| Navigation and modals | Transitions, backdrops, optimistic close on Escape or a backdrop click. | The request to close; the end of a transition. |
| Switch, activity indicator, toasts, error reports | Drawn and animated by the host. | A toggle. |

A slow PHP handler delays the next frame of *content*; it does not stall an animation, a scroll or the caret.

## Threading

| Thread | Work |
| --- | --- |
| UI thread | The platform's main loop, the runtime, the engine, the scene, every controller and every `Backend` call. |
| `pam-worker-read` | Reads worker messages and posts them to the inbox. |
| `pam-worker-write` | Writes to the worker, coalescing whatever is queued into one write. |
| `pam-watch` | Development only: polls the project for changes. |
| `pam-timers` | Sleeps until the next deadline. Started with the first timer. |
| `pam-device` | Samples the device status while a watch is open. |
| `pam-storage`, `pam-files`, `pam-sqlite`, `pam-secure-storage` | One serial thread each, so results arrive in call order. |
| `pam-http`, `pam-http-transfer`, `pam-files-download`, `pam-image`, `pam-image-prefetch`, `pam-media-cache` | Bounded pools for network work. |
| Backend threads | Image decoding; on Linux the X11 hotkey listener and the tray service. |

Rules:

- Nothing but the UI thread touches the scene or the backend. Other threads post to the **inbox**, which wakes the UI thread.
- Callbacks handed to a backend (`clipboard_text`, `show_alert`, `popup_menu`, `pick_files`) run later, on the UI thread.
- Blocking work never runs on the UI thread: disk, network, SQLite, the keychain and image decoding all have their own threads.
- Pool threads start on demand and exit after 30 seconds without work, so an idle pool holds no thread.

## Memory and idle rules

An application that is doing nothing costs nothing:

- **No frame clock while nothing animates.** The runtime asks the backend for a frame only while the animator has something running.
- **One wake-up timer.** Every deadline of the core (long press, debounce, media progress, releases) shares a single `schedule_wake`, armed only when the earliest deadline changes.
- **PHP sleeps** in a blocking read between turns.
- **The file watcher exists only in development.**

Bounded memory:

- A decoded image is no larger than the first of 128, 256, 512, 1024 or 2048 pixels that covers the box showing it.
- Decoded images that no element shows stay in a least-recently-used set of at most 48 images and 96 MiB, then are released.
- A media player that is not visible pauses at once and releases its decoder after 5 seconds; a player that lost its visible-autoplay grant, after 1.5 seconds. Both resume at the position they had.
- The media disk cache is trimmed to its budget (16 to 256 MiB), sparing pinned files and the newest one.
- A list mounts only the rows near its viewport.
- Frames, messages, module payloads and query results all have explicit size limits; a value over a limit fails instead of growing a buffer.

## Errors

An uncaught PHP error is sent to the host as an Error message with its phase (boot, render, event, module callback, promise, hot reload), the source around the failing line and the call stack. What the host shows depends on the mode; see [Development](development.md#the-error-overlay).

A frame the engine rejects does not crash anything: the host clears the scene, recreates the engine and asks PHP for a complete tree.

## Related

- [Platforms](platforms.md) for the three backends.
- [Getting started](getting-started.md) for the command line and the environment.
- [Modules](modules.md) for the services behind module calls.
