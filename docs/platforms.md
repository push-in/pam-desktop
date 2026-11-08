# Platforms

PAM Desktop has one render core and three backends. Each backend implements the same [`Backend` contract](architecture.md#the-backend-contract); everything above it (layout, interaction, animation, navigation, modules) is shared.

## Status

| System | Backend crate | Status |
| --- | --- | --- |
| Linux | `pam-desktop-gtk` (GTK 4) | The reference backend. Every application under `examples/` was run and checked on Linux, and every feature added since was run there first. |
| macOS | `pam-desktop-appkit` (AppKit) | Run on macOS 26.7.1 (Tahoe, x86_64) in a virtual machine without a GPU: the core examples, hot reload, input, a real application and a DMG built by `pam-desktop build`. Not run on Apple Silicon or on real Mac hardware. |
| Windows | `pam-desktop-win32` (Win32, Direct2D, DirectWrite) | Written against the `Backend` contract and type-checked by cross compilation (most recently for `x86_64-pc-windows-gnu`). Not run on Windows yet. |

What this means in practice:

- **Linux is what you can rely on today.**
- **macOS runs**, with the gaps listed under [macOS](#macos). Expect differences on Apple Silicon and with a GPU, which were not tried.
- **Windows has never run.** Type-checking proves that the code agrees with the platform bindings, not that it behaves. The first runs are expected to need fixes. [First run on macOS and Windows](first-run-macos-windows.md) is the checklist for those runs and the list of known differences from the GTK backend.
- **The host starts the backend of the system it was built for:** `pam_desktop_gtk::run` on Linux, `pam_desktop_appkit::run` on macOS and `pam_desktop_win32::run` on Windows. The three crates export the same entry point, `run(application_id, config)`.
- **Two things were not verified on Linux either:** the accessibility tree with a real screen reader, and global shortcuts through the Wayland portal. Global shortcuts with X11 key grabs were verified.

### Recent features by system

"Yes" was run and checked on that system. "Written" means the code is in the backend and type-checks, but has not been executed there. "No" means the backend does not do it.

| Feature | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Hot reload (`pam-desktop dev`) | Yes, about 315 ms from save to window | Yes | Written |
| `pam-desktop init`, `dev` without arguments | Yes | Not verified (`dev examples/counter` ran) | Written |
| PHP found or installed by the host | Yes (PAM runtime) | Yes (PAM 2.2.9 installed with no PHP on the machine) | Written (windows.php.net PHP 8.5) |
| `pam-desktop build` | Yes | Yes: directory and DMG of an x86_64 release host | Written |
| `font-family` lists, packaged fonts | Yes | Yes (packaged icon fonts) | Written |
| Inline boxes in nested text | Yes | Written | Written |
| `overflow-wrap`, `word-break` | Yes | No | Written |
| `hyphens: auto` | Yes | No | No |
| Scrollbar gutter, width and colour | Yes | Overlay scrollers that keep the content width verified; the classic styled thumb is written | Written |
| `backdrop-filter`, modal and toast backdrop blur | Yes | Written (Core Image), not verified on screen | Written (Direct2D) |
| `opacity: 0` stays clickable | Yes | Not verified | Not verified |
| `pointerEvents` in native hit testing | Yes | Written | Written |
| Typed drag and drop, `captureKeys` | Yes | Written | Written |
| Canvas paths, gradients, text; charts | Yes | Written | Written |
| Printing | Print to File and its PDF checked; no physical printer | Written | Written |
| WebSocket, `Files::save`, `SQLite::transactionIf` | Yes | Written | Written |

## Linux

### Dependencies

| Package (Debian and Ubuntu names) | Needed for |
| --- | --- |
| `libgtk-4-dev` | The backend. GTK 4.6 or newer. |

At run time the application uses what GTK uses: Pango for text, gdk-pixbuf for image decoding (the formats are those of the installed loaders) and GStreamer, through `GtkMediaFile`, for media.

### Running a released host

The host that `vendor/bin/pam-desktop` downloads is built on Ubuntu 22.04, for x86_64 and arm64, against GTK 4.6. The GTK functions newer than 4.6 (alert and file dialogs, URI and file launchers, masks, path strokes, screen reader announcements and others) are looked up when first used and used when the installed GTK has them; on an older GTK the backend takes a GTK 4.6 path instead (for example a `GtkFileChooserNative`, or Cairo for dashed borders). It needs:

| Requirement | Debian and Ubuntu package |
| --- | --- |
| glibc 2.35 or newer | `libc6` |
| GTK 4.6 or newer | `libgtk-4-1` |
| WebKitGTK 6, only for applications that show a `WebView` | `libwebkitgtk-6.0-4` (Ubuntu 24.04 and later) |
| GStreamer plugins, for media | `gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good` |

A host built from a checkout needs what the machine that built it had.

### Optional features

The host crate (`crates/pam-desktop-host/Cargo.toml`) has four cargo features, all on by default. Three of them are the optional parts of the backends:

| Feature | Provides | Extra system library |
| --- | --- | --- |
| `webview` | Windows: the `WebView` element, through WebView2. | none |
| `tray` | Linux: the tray icon, as a StatusNotifierItem on the session bus. | none |
| `hotkeys` | Linux: global shortcuts, as X11 key grabs or through the desktop portal on Wayland. | none |
| `sqlite` | The [SQLite module](modules.md#sqlite). SQLite is compiled from C. | none |

On Linux the `WebView` element is always built: WebKitGTK 6 is loaded from the system the first time one is shown, and a missing library is an error reported through `onError`. On Windows, without `webview`, a `WebView` element reports an error through `onError`. Without `tray`, `Tray::show()` does nothing. Without `hotkeys`, shortcuts registered with `global: true` are not bound; local shortcuts and menu shortcuts are handled by the core and keep working.

A leaner host is built by naming the features to keep:

```bash
cargo build -p pam-desktop-host --no-default-features --features tray,sqlite
```

On macOS the tray, global shortcuts and `WKWebView` are always built.

### Wayland and X11

GTK chooses the display server; the application code is the same. The differences that reach an application:

| Topic | X11 | Wayland |
| --- | --- | --- |
| Global shortcuts | Keys are grabbed on the root window. A combination that another application already holds is refused by the server. **Verified.** | Applications cannot grab keys. The backend binds them through the GlobalShortcuts desktop portal, with the consent of the user. **Not verified.** |
| `Window::center()`, `alwaysOnTop()` | Sent to the window manager (a position request and the `_NET_WM_STATE_ABOVE` state). **Verified** with Xfwm. | Do nothing: Wayland gives applications no say on placement or stacking. |

`Window::progress()` is sent on the session bus as a `com.canonical.Unity.LauncherEntry` update, the launcher entry API that the Ubuntu Dock, Dash to Dock and the Plasma task manager listen to. The update names the desktop entry `<applicationId>.desktop`, so it reaches an installed application, not a loose `pam-desktop run`. A desktop that does not listen shows nothing.

### Global shortcuts

A shortcut registered with `global: true` fires while another application has the focus. On X11 a small helper thread owns the grabs; Caps Lock and Num Lock do not hide a shortcut. On Wayland the binding is made by the portal, with the consent of the user.

Application shortcuts (`Shortcut::register()` without `global`) do not depend on any of this.

### Tray icon

GTK 4 has no tray icon of its own. The backend registers a StatusNotifierItem on the session bus, the protocol that KDE, Cinnamon, XFCE and the GNOME AppIndicator extension display.

- The desktop must run a host for that protocol. On GNOME that is the AppIndicator extension.
- A desktop without a host does not fail the application: `Tray::show()` succeeds and the icon appears if a host starts later.
- With no icon file given, the themed icon of the application is used.

### Other notes

- **Secure storage** needs a Secret Service in the session. Without one every `SecureStorage` call fails; nothing is stored in the clear.
- **Notifications** go through the GTK application, named by the `applicationId`.
- **File dialogs** are GTK's. `scripts/shot.sh` sets `GDK_DEBUG=no-portals` and `GTK_USE_PORTAL=0` so that dialogs stay on the virtual display during captures.
- **Links and files** open with `xdg-open`.
- **Dark mode** is the `color-scheme` setting of the desktop portal (`org.freedesktop.appearance`), which GNOME, KDE and others publish. A desktop without the portal, or a run with `GTK_USE_PORTAL=0`, falls back to the GTK settings: a theme whose name contains "dark", or the `gtk-application-prefer-dark-theme` setting. **Text scale** and **reduced motion** are the GTK settings `gtk-xft-dpi` and `gtk-enable-animations`. All three are followed while the application runs.
- **Screen capture protection** (`Screen::secure()`) is not enforced: the GTK backend does not implement it and the call reports `false`.
- **Single instance.** Without `singleInstance` each launch is its own process: the GTK application is registered as non-unique. With it the GTK application is unique: a second launch forwards its link to the first, which presents its window, and exits. See [Distribution](distribution.md#links-and-single-instance).

## macOS

**Run in a virtual machine.** macOS 26.7.1 (Tahoe) x86_64 without a GPU, Xcode 26.6, PHP 8.5.12 from PAM 2.2.9. [What was verified on macOS](first-run-macos-windows.md#what-was-verified-on-macos) lists what ran, what was fixed and what was not tried.

Design of `pam-desktop-appkit`:

- Every scene node is a layer-backed `NSView`. A box that is only a solid, uniformly rounded and bordered rectangle is painted by its layer's own properties and gets no backing store; anything richer is drawn with Core Graphics.
- Text is typeset with TextKit, shared by measurement, drawing, hit testing and selection.
- Scrolling uses `NSScrollView`; text entry uses `NSTextField`, `NSSecureTextField` and `NSTextView`.
- Images are decoded with ImageIO off the main thread.
- Media uses `AVPlayer`, shown by an `AVPlayerLayer`, or by AVKit's `AVPlayerView` when the application asks for the system's transport controls.
- Web pages use `WKWebView`.
- The shell uses the system menu bar, `NSStatusItem` for the tray, open and save panels, `NSAlert` and the notification center.
- The frame clock is a display link that is paused between frames.
- Accessibility maps to `NSAccessibility`, including custom actions.
- Links arrive through `application:openURLs:`, at launch and later. The backend does not read `singleInstance`.
- `Window::progress()` is a badge on the Dock icon; `Screen::secure()` sets the sharing type of every window to none.
- The appearance and Reduce Motion are followed while the application runs. The text scale of the system is always 1.
- The host uses the system TLS stack and the Keychain.
- Packaged fonts are registered with CoreText. Scroll views keep overlay scrollers; with `scrollbar-gutter: stable` and a styled bar the backend draws a classic thumb in the gutter. The focus ring shows only after keyboard input, like `:focus-visible`.
- Text wraps at word boundaries only: `word-break`, `overflow-wrap` and `hyphens` are not applied.
- `PAM_DESKTOP_CAPTURE=<file.png>` writes the first window to a PNG from inside the process (macOS only).
- Without PHP on the machine, the host installs the PAM runtime and runs the worker with `pam exec`.

[First run on macOS and Windows](first-run-macos-windows.md#macos-pam-desktop-appkit) lists what to check first and the known differences from the GTK backend.

## Windows

**Unverified: written and type-checked, never run.**

PHP on Windows: PAM has no Windows release, so the host uses `PAM_DESKTOP_PHP`, the PHP it installed before, `php.exe` on `PATH`, and otherwise downloads the newest official PHP 8.5 build (non thread safe, x64) from windows.php.net into `%LOCALAPPDATA%\PAM Desktop\php\<version>`, after checking its published SHA-256. It needs the Visual C++ Redistributable 2015-2022 (x64); the host says so when `vcruntime140.dll` is missing. See [Distribution](distribution.md) for what a bundle ships.

Design of `pam-desktop-win32` (Windows 10 version 1809 and later, and Windows 11):

- One top-level window per application window, per-monitor DPI aware, rendered as a whole with Direct2D and DirectWrite on a flip-model swap chain.
- Views are a retained tree of plain structs, not child windows. Only text inputs (a child `EDIT` control) and web views are child windows, placed over their node.
- A frame is rendered only when something changed.
- Images are decoded with WIC on worker threads and kept as Direct2D bitmaps.
- Media uses a Media Foundation media engine in frame-server mode: each video frame lands in a Direct3D texture drawn inside the Direct2D scene, so video is clipped, rounded and overlaid like the rest of the tree.
- Web pages use WebView2, the Evergreen runtime of Edge. A missing runtime is reported through `onError`. `webview` is a cargo feature of the crate, on by default and forwarded by the `webview` feature of the host.
- The shell uses the window menu bar with accelerators, the notification area for the tray, the system file dialogs, global hot keys and OLE drag and drop.
- Accessibility is a UI Automation provider tree over the retained views.
- With `singleInstance`, a named mutex tells a second launch that the application runs; the second launch hands its link to the first with `WM_COPYDATA` and exits.
- `Window::progress()` is the progress of the taskbar button. `Screen::secure()` excludes every window from captures (`WDA_EXCLUDEFROMCAPTURE`), or shows it black where the system only supports that.
- Dark mode, text scale and the animation setting are read again on `WM_SETTINGCHANGE`.
- Menus have no public dark mode on Windows. In the dark appearance, popup and context menus follow the application mode of the theme engine, set through functions `uxtheme.dll` exports by ordinal only (Windows 10 1809 and later), and the window draws its own menu bar.
- A release build of the host is a windowed program, so starting an application opens no console. Started from a terminal, the host attaches to the console of its parent, so `build`, `init` and the other commands still print. A debug build is a console program.
- The host embeds an application manifest: version 6 of the common controls, per-monitor DPI awareness, long paths and UTF-8 as the code page of the process.
- The host uses the system TLS stack and the Credential Manager.
- Packaged fonts are loaded into a DirectWrite collection, and as private GDI fonts for the `EDIT` controls. Inline boxes are DirectWrite inline objects. `scrollbar-width: none` hides the bar; a reserved gutter shows a classic bar. Line breaking follows `word-break` and `overflow-wrap`; `hyphens` is not applied.
- Printing uses the system print dialog and spools with GDI on a worker thread.

[First run on macOS and Windows](first-run-macos-windows.md#windows-pam-desktop-win32) lists what to check first and the known differences from the GTK backend.

## Cross-checking macOS and Windows from Linux

The two backends can be type-checked on a Linux machine. Install the targets once:

```bash
rustup target add aarch64-apple-darwin x86_64-pc-windows-msvc x86_64-pc-windows-gnu
```

Then:

```bash
cargo check -p pam-desktop-appkit --target aarch64-apple-darwin
cargo check -p pam-desktop-win32 --target x86_64-pc-windows-msvc
```

The host itself can be type-checked for Windows with MinGW installed (`cargo check -p pam-desktop-host --target x86_64-pc-windows-gnu`) and for macOS without its default features (`cargo check -p pam-desktop-host --target aarch64-apple-darwin --no-default-features`), since SQLite needs the Apple SDK to compile.

`cargo check` compiles without linking. It catches type errors against the platform bindings and against the `Backend` trait. It does not catch a wrong selector, a wrong message, a layout that is off by a pixel or anything else that only shows when the code runs.

The `sqlite` feature of the host compiles SQLite from C. A build of the host that only type-checks for another system turns it off.

The CI workflow has a job that builds and tests the workspace on a macOS runner and on a Windows runner; see [Development](development.md#continuous-integration). The workflow has not run yet.

## Related

- [Getting started](getting-started.md) for requirements and the command line.
- [Accessibility](accessibility.md#what-each-backend-maps) for the accessibility mapping of each backend.
- [Architecture](architecture.md) for the contract the backends implement.
- [Distribution](distribution.md) for packaging.
