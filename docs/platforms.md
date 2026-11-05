# Platforms

PAM Desktop has one render core and three backends. Each backend implements the same [`Backend` contract](architecture.md#the-backend-contract); everything above it (layout, interaction, animation, navigation, modules) is shared.

## Status

| System | Backend crate | Status |
| --- | --- | --- |
| Linux | `pam-desktop-gtk` (GTK 4) | The reference backend and the only one exercised so far. Every application under `examples/` was run and checked on Linux. |
| macOS | `pam-desktop-appkit` (AppKit) | Written against the `Backend` contract and type-checked by cross compilation. Not run on a real machine yet. |
| Windows | `pam-desktop-win32` (Win32, Direct2D, DirectWrite) | Written against the `Backend` contract and type-checked by cross compilation. Not run on a real machine yet. |

What this means in practice:

- **Linux is what you can use today.** The `pam-desktop` binary starts the GTK backend.
- **macOS and Windows have never run.** Type-checking proves that the code agrees with the platform bindings, not that it behaves. The first runs on real machines are expected to need fixes. At the time of writing the host binary does not start these backends: on macOS and Windows `pam-desktop` exits with "this platform is not supported yet". Both crates export the same entry point as the GTK one, `run(application_id, config)`.
- **Two things were not verified on Linux either:** the accessibility tree with a real screen reader, and global shortcuts through the Wayland portal. Global shortcuts with X11 key grabs were verified.

## Linux

### Dependencies

| Package (Debian and Ubuntu names) | Needed for |
| --- | --- |
| `libgtk-4-dev` | The backend. GTK 4.14 or newer. |
| `libwebkitgtk-6.0-dev` | The `webview` feature. |

At run time the application uses what GTK uses: Pango for text, gdk-pixbuf for image decoding (the formats are those of the installed loaders) and GStreamer, through `GtkMediaFile`, for media.

### Optional features

The backend crate has three cargo features, all on by default:

| Feature | Provides | Extra system library |
| --- | --- | --- |
| `webview` | The `WebView` element, through WebKitGTK 6. | `libwebkitgtk-6.0` |
| `tray` | The tray icon, as a StatusNotifierItem on the session bus. | none |
| `hotkeys` | Global shortcuts: X11 key grabs, or the desktop portal on Wayland. | none |

Without `webview`, a `WebView` element reports an error through `onError`. Without `tray`, `Tray::show()` does nothing. Without `hotkeys`, shortcuts registered with `global: true` are not bound; local shortcuts and menu shortcuts are handled by the core and keep working.

The features belong to `crates/pam-desktop-gtk/Cargo.toml`. The host depends on the backend with its default features, so building without one means changing that dependency in the workspace manifest (`default-features = false` and the features you keep).

### Wayland and X11

GTK chooses the display server; the application code is the same. The differences that reach an application:

| Topic | X11 | Wayland |
| --- | --- | --- |
| Global shortcuts | Keys are grabbed on the root window. A combination that another application already holds is refused by the server. **Verified.** | Applications cannot grab keys. The backend binds them through the GlobalShortcuts desktop portal, with the consent of the user. **Not verified.** |

`Window::center()`, `alwaysOnTop()` and `progress()` do nothing on either: Wayland gives applications no say on placement, stacking or taskbar progress, and GTK 4 exposes none of them.

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
- **Dark mode, text scale and reduced motion** are read from the GTK settings.
- Each launch is its own process: the GTK application is registered as non-unique.

## macOS

**Unverified: written and type-checked, never run.**

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
- The host uses the system TLS stack and the Keychain.

## Windows

**Unverified: written and type-checked, never run.**

Design of `pam-desktop-win32` (Windows 10 version 1809 and later, and Windows 11):

- One top-level window per application window, per-monitor DPI aware, rendered as a whole with Direct2D and DirectWrite on a flip-model swap chain.
- Views are a retained tree of plain structs, not child windows. Only text inputs (a child `EDIT` control) and web views are child windows, placed over their node.
- A frame is rendered only when something changed.
- Images are decoded with WIC on worker threads and kept as Direct2D bitmaps.
- Media uses a Media Foundation media engine in frame-server mode: each video frame lands in a Direct3D texture drawn inside the Direct2D scene, so video is clipped, rounded and overlaid like the rest of the tree.
- Web pages use WebView2, the Evergreen runtime of Edge. A missing runtime is reported through `onError`. `webview` is a cargo feature of the crate, on by default.
- The shell uses the window menu bar with accelerators, the notification area for the tray, the system file dialogs, global hot keys and OLE drag and drop.
- Accessibility is a UI Automation provider tree over the retained views.
- The host uses the system TLS stack and the Credential Manager.

## Cross-checking macOS and Windows from Linux

The two backends can be type-checked on a Linux machine. Install the targets once:

```bash
rustup target add aarch64-apple-darwin x86_64-pc-windows-msvc
```

Then:

```bash
cargo check -p pam-desktop-appkit --target aarch64-apple-darwin
cargo check -p pam-desktop-win32 --target x86_64-pc-windows-msvc
```

`cargo check` compiles without linking. It catches type errors against the platform bindings and against the `Backend` trait. It does not catch a wrong selector, a wrong message, a layout that is off by a pixel or anything else that only shows when the code runs.

The `sqlite` feature of the host compiles SQLite from C. A build of the host that only type-checks for another system turns it off.

## Related

- [Getting started](getting-started.md) for requirements and the command line.
- [Accessibility](accessibility.md#what-each-backend-maps) for the accessibility mapping of each backend.
- [Architecture](architecture.md) for the contract the backends implement.
- [Distribution](distribution.md) for packaging.
