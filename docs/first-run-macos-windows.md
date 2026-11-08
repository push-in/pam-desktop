# First run on macOS and Windows

The AppKit and Win32 backends were written on Linux and type-checked by cross compilation. The AppKit backend has since run on macOS (see [What was verified](#what-was-verified-on-macos)); the Win32 backend has not run on a real machine yet (it was cross-compiled and run under Wine, see [What was checked before the first run](#what-was-checked-before-the-first-run)). This page is the order in which to look at things on the first run, the behavior that is known to differ from the GTK backend, and what was left out.

Linux is the reference: every example under `examples/` was run and checked there.

## Before anything else

1. Build and run in **debug** first: `cargo build -p pam-desktop-host`, then `target/debug/pam-desktop dev examples/counter`.
   - macOS: objc2 checks the signature of every Objective-C method the backend overrides when its classes are registered. A mismatch is a panic with the name of the method, at startup.
   - Windows: the backend keeps its state in a `RefCell`. A re-entrant borrow is ignored in release and is a `debug_assert` in debug.
2. Then walk through the examples in this order, which goes from the least to the most platform code: `counter`, `controls`, `lists`, `text`, `images`, `motion`, `navigation`, `shell`, `windows`, `dragdrop`, `canvas`, `media`, `webview`, `links`, `showcase`.
3. `PAM_DESKTOP_DEBUG=1` prints every event the host sends to PHP.
4. Two things the core does with ordinary views were only seen on Linux. The sliding line at the bottom of `examples/text` is a text view moved inside a clipping view: check that it is clipped at the padding of its box and slides smoothly. The blue card of `examples/navigation` flies to its route as two copies that are scaled and faded: check that the copies stretch from the top-left corner of the card, not from its center.

## macOS (`pam-desktop-appkit`)

### What was verified on macOS

macOS 26.7.1 (Tahoe, x86_64, a virtual machine without a GPU), Xcode 26.6, Rust stable, PHP from PAM 2.2.9. Windows of 1200×800, seen on the real screen and driven with real keyboard and pointer events (the virtual machine's USB keyboard and tablet).

- **Startup and PHP.** No objc2 signature panic. With no PHP on the machine the host installed PAM 2.2.9 with its installer and ran the worker with `pam exec`.
- **Examples:** `counter`, `controls`, `lists` (2000 rows), `text`, `images`, `motion`, `navigation` render like on Linux, in light and dark appearance. `showcase` needs `composer install` in its directory first.
- **Hot reload:** `pam-desktop dev examples/counter` re-rendered an edit of `index.php` while running.
- **Input:** clicks; typing with masks (phone, currency) and the value reaching PHP; Tab and Space; triple click and Command-C into the pasteboard; Command-Q through the default menu; wheel scrolling and `scrollTo` in a 2000-row list; a modal dialog closed with Escape; a bottom sheet; dragging the window by its title bar.
- **Painting:** outer shadows fall downward like on Linux (the pixels around a shadowed card match the GTK capture), so `LAYERS_Y_DOWN` is right.
- **A real application:** see [Validation with a real application](#validation-with-a-real-application).
- **Bundle:** `pam-desktop build --format directory --format dmg` with a release host (x86_64); the `.app` from the 19 MB DMG opens. Idle at its first screen for 30 s: 0.0% CPU, 41 MB resident for the host and 41 MB for the PHP worker.
- **Fixed on the first run:**
  - Text typed into a single-line input never reached the core: attaching the field removed every notification registration of its delegate, including the `controlTextDidChange:` one `setDelegate:` had just made.
  - Every click on a focusable box left the system focus ring on it; the ring now shows only after keyboard input, like `:focus-visible` in GTK.
  - Icon fonts (`asset://…ttf`) drew a missing-glyph box: the backend now loads packaged fonts from the asset root (`fonts.rs`).
  - With a mouse AppKit switches scroll views to legacy scrollers, which took 15 points from content laid out for the whole viewport: scroll views keep overlay scrollers.
  - An input without a placeholder colour used the system's hint colour: it is now the text colour at 45%, like GTK.
  - A bundle made with PAM shipped the `pam-run` launcher script instead of the `pam` binary and its `libphp.dylib`, so it could not start: the build now ships both (`pam_binary`, `copy_runtime_libraries` in `packager.rs`).
- **Second run (protocol 2028):** the PDV's 59 visual fixtures (every `*-visual.php`, the `group-d` views and states, `wallet-marketing-visual.php` for each route) were captured on the real screen and compared with Linux captures of the same snapshot. Layout matches; the differences are fonts, the system appearance (the virtual machine is in dark mode, so default window and button colours are dark), the network (Linux captures run offline) and the effects below. Also exercised: a context menu and its choice (`examples/shell`), drag and drop between columns (`examples/dragdrop`), video playback (`examples/media`, 7% CPU in software rendering), messages both ways (`examples/webview`), the window toasts above a modal (`examples/toast` and a form dialog of the PDV), printing with its preview and *Save as PDF* (the receipt's text is in the PDF), the password toggle keeping the caret, and inline boxes (`examples/text`). Idle: 0% CPU for the host and the PHP worker, at rest, after ten interactions and minimized.
- **Fixed on the second run:**
  - Inline boxes of nested texts (`TextSpanBoxes`) are drawn: each edge is an invisible character whose kern makes its advance the padding plus border (the line breaker sees it, the line height does not change), and the box of each line is painted under the text from TextKit's glyph positions; selection, hit testing and copying map the offsets back to the paragraph.
  - `scrollbar-gutter: stable` with `scrollbar-width`/`scrollbar-color`: a classic bar drawn in the gutter (thumb at least 40 points, `thin` is a 6-point thumb inset 2 points), shown only while the content overflows, draggable; without a gutter the overlay scroller stays.
  - Showing or hiding a password kept the caret on GTK but selected the whole value on AppKit (the field is replaced by another class and AppKit selects all of a field that takes focus): the selection is put back, also after the event.
  - An application whose root is only a modal showed an empty window: the root and its overlay were the same view, stacked twice.
  - Printing aborted when the print panel drew its preview: the preview's drawing context is a proxy that forwards `CGContext` without declaring it, and the checked message panicked inside `drawRect:`.
  - Font family lists go through the shared parser (`font_family.rs`), so `BlinkMacSystemFont` and the other generic names map to the system fonts.
  - The default Edit menu's Copy and the alert's OK come from the shared catalog (`i18n.rs`).
- `PAM_DESKTOP_CAPTURE` writes the window to a PNG from inside the process, for a locked or headless session. It leaves out layer shadows, masks and Core Image filters.

Known issue outside this repository: `pam` up to 2.2.9 refuses to run anything in a project whose packages are Composer path repositories installed as symlinks to a checkout outside the project (`PAM-E005 … target escapes the project`), which is how an application is developed against a checkout of this repository. The fix is in the PAM repository and needs a release; until then install the packages as copies (`"symlink": false`) or set `PAM_DESKTOP_PHP` to another PHP.

### Validation with a real application

The macOS run also used a point-of-sale application built on PAM Desktop (PushinShop PDV) as a test case: login with typing, the sales screen, a table list with scrolling, a product editor with its category popup, and a wallet screen. The layout matched the Linux build to the pixel at card and column edges; the remaining differences came from fonts, as they do for the application's Electron version, since both ask for the system font. The run is recorded in `evidence/macos-runtime-validation.json` of that application's repository. Nothing in this repository depends on it.

### Not verified on macOS

Not verified yet: `blur`, `backdropBlur` and `colorMatrix` on a Mac with a GPU (the virtual machine has no Metal device; see below), notifications actually shown (from a `.app` the call runs and the menu of the menu bar works, but no banner appeared in the virtual machine, unsigned or signed ad hoc), signing, and Apple Silicon.

### What to check first, by risk

1. **Orientation of layers and shadows.** Two constants hold the assumption: `LAYERS_Y_DOWN` in `view.rs` (transforms, layer shadows, masks) and `SHADOW_SPACE_Y_DOWN` in `paint.rs` (text shadow, inner shadow). If something is mirrored vertically, flip the constant.
2. **Transforms.** Translation goes into the frame; scale and rotation go into the layer, with a key-value observer that puts them back when AppKit resets the layer. Check scale and rotation animations and the first time a view is shown (`examples/motion`).
3. **macOS before 14.** Visible overflow and shadows rely on `clipsToBounds = false`; older systems may clip.
4. **Pointer.** The overridden `sendEvent:`, the synthesized mouse-up after a native tracking loop, and the title strip of an undecorated window, which may still drag natively.
5. **Focus.** Tracking through `makeFirstResponder:` with the field editor, `selectOnFocus`, the caret color.
6. **Switching between layer painting and `drawRect:`** when a box becomes more than a solid rounded rectangle.
7. **TextKit.** Ellipsis with `maxLines`, vertical centering with a fixed `lineHeight`, measurement equal to drawing, font resolution.
8. **Default menus** and the interception of the editing shortcuts (Command-C, V, X, A, Z).
9. **Scrolling.** Pending offsets (an inverted list opens at the bottom), nested scroll views, overlay scrollers.
10. **Display link** pausing and resuming; wake-ups while a menu is open and during a live resize.
11. **AVFoundation** (the key-value observation thread) and the Core Image filter keys of `blur` and `colorMatrix`.
12. **Wheel units, gesture signs, tooltips and cursor rectangles** over overlapping views.
13. **Teardown.** A view or window destroyed from inside one of its own callbacks is released on the next turn of the run loop. That does not cover a nested run loop: remove a list while dragging its scroller, close a window from a menu that is open.
14. **Window drag** of an undecorated window starts from a mouse-dragged event (`performWindowDragWithEvent`): check that it starts.
15. **Context menu choice** is read one turn after the menu closes: a choice that arrives later is reported as cancelled.
16. **Animated `scrollTo`** uses an implicit animation; if it jumps instead, only the animation is lost.
17. **A frame requested while the window is hidden or minimized** arrives when the window comes back (the display link of a view does not fire, macOS 14 and later).
18. **SVG images** (an `Image` with an `.svg` file or SVG bytes): they are drawn by `NSImage` on a worker thread and rasterized with `CGImageForProposedRect:`. Check that one shows, that it is sharp at the size of its box, and that it is not upside down.
19. **Media `Center` fit** (`examples/media` with `->fit(ImageFit::Center)` and a video smaller than its box): the surface view is resized to the video and centered once the player reports its size. Check that it re-centers when the window is resized and when the source changes.

### Known differences from the GTK backend

Menus and shortcuts:

- An empty menu bar installs the standard App, Edit and Window menus instead of removing the bar: without them there is no Command-Q.
- When the first menu holds no application roles, an application menu is inserted before it.
- `About` opens the standard panel; `Preferences` reports `role:preferences`.
- Quit (menu, Dock, Command-Q) asks the application like closing the main window does. A logout is cancelled until the application quits.
- Global shortcuts fire only while the application is in the background and need the Accessibility permission.

Focus, keyboard and pointer:

- `FocusChanged` is reported for the node that holds the first responder only (GTK also reports ancestors).
- A click does not focus a focusable box; Tab and `focus()` do.
- Keys consumed by a text control are not reported, except Escape and chords without a binding.
- Control-click is the secondary button.
- One wheel notch is 100 units, assuming a delta of 1.0 per notch.
- A three-finger swipe to the left is `BackRequested`.

Inputs:

- `purpose` and `capitalization` only turn completion, spelling and substitutions off.
- `maxLength`: the single-line field truncates after the edit; the editor rejects the edit.
- The vertical alignment of the multi-line editor is always top.

Painting:

- Outer shadows are layers on a sibling view below the node.
- A simple box is painted by its layer; its border is then above its children.
- `blur`, `backdropBlur` and `colorMatrix` use Core Image filters, which Core Animation runs on the GPU. Without a Metal device (a virtual machine) a filtered layer renders empty, and so does everything a backdrop filter covers: the view is then shown without the filter.
- Inline boxes (`TextSpanBoxes`) reserve their edges with characters of the box text's font and paint one box per line under the text, like GTK.
- A styled scrollbar in a reserved gutter is a classic bar drawn by the backend (a rounded thumb, at least 40 points long); elsewhere the system's overlay scroller.
- A conic gradient is a fan of 360 slices.
- Text wraps at word boundaries only: `word-break`, `overflow-wrap` and `hyphens` are not applied.
- Scroll views always use overlay scrollers. With `scrollbar-gutter: stable` and a styled bar, a classic thumb is drawn in the gutter (grey without `scrollbar-color`).

Images, media and web:

- EXIF orientation is applied. An SVG document is drawn by `NSImage` (macOS 10.15 or newer) at the size it is shown at, or at twice its natural size when that is not known.
- Media `Center` and `Repeat` show a video smaller than the box at its own size, in the middle, like the GTK backend. With `controls()` the player view always fills the box.
- `domStorage: false` uses a non-persistent data store; changing `domStorage` or `inlineMedia` recreates the view and reloads the page.

System:

- Notifications need a bundle identifier: a loose binary posts none.
- `Window::progress()` is a Dock badge ("42%").
- A modal window with a parent is a sheet.
- Links arrive through `application:openURLs:`, also the one that launched the application, so `Linking::listen()` receives it and `Linking::initial()` may be empty.
- Battery, network and Low Power Mode are read with `pmset`, `route` and `networksetup`, at most every 10 seconds.
- `Share::text()` and `Share::files()` open the sharing service picker at the pointer, or in the middle of the window when the pointer is outside it.
- `Screen::secure()` sets the sharing type of every window to none. Capture tools built on ScreenCaptureKit may still record the window on recent systems.
- Reduce Motion is followed while the application runs; macOS has no system text scale for applications.

## Windows (`pam-desktop-win32`)

### First test on a real Windows machine

For any application. Windows 10 1809 or later, or Windows 11, x64.

1. **Tools.** Rust with the MSVC toolchain (`rustup` default on Windows; it
   needs the Visual Studio Build Tools with "Desktop development with C++"),
   Git, Composer, and the Visual C++ Redistributable 2015-2022 x64
   (<https://aka.ms/vs/17/release/vc_redist.x64.exe>; the official PHP refuses
   an older one). WebView2 comes with Windows 11 and current Windows 10.
2. **Build the host** in a Developer PowerShell, from the checkout:
   `cargo build -p pam-desktop-host`. Run `target\debug\pam-desktop.exe --version`.
3. **PHP.** Do nothing: the first `dev`, `run` or `build` without
   `PAM_DESKTOP_PHP` downloads the official PHP 8.5 NTS x64 from
   windows.php.net, checks its published SHA-256, and installs it under
   `%LOCALAPPDATA%\PAM Desktop\php\<version>` (it prints both lines). To use
   another PHP 8.5, set `PAM_DESKTOP_PHP` to its `php.exe`. Extensions beyond
   the common set are listed in `pam-desktop.json` (`"extensions": ["auto", "gmp"]`).
4. **Examples**, in the order of the list above, with
   `$env:RUST_BACKTRACE=1; target\debug\pam-desktop.exe dev examples\counter`.
   On each: click, type into every input (the value must reach PHP; in
   `controls`, "PHP sees" follows the masked phone), Tab through the controls
   (the focus ring shows only after Tab, never after a click), scroll with the
   wheel and by dragging the bar, resize the window, switch Windows between
   light and dark.
5. **A bundle**: `target\debug\pam-desktop.exe build examples\counter --format directory --format portable`
   (add `--format msi` with WiX 4 installed, `--format msix` with the Windows
   SDK). Copy the directory to another folder, ideally another machine
   without PHP or the redistributable, and start `bin\<name>.exe`.
   Then build with `cargo build --release -p pam-desktop-host` and repeat: a
   release host has no console.
6. **Logs.** From a terminal, `$env:PAM_DESKTOP_DEBUG=1` prints every event
   sent to PHP; PHP errors go to the same terminal. Save the output with
   `... 2>&1 | Tee-Object run.log`. For a crash: `RUST_BACKTRACE=1` and the
   panic text; for a bundle that does not open, start `bin\<name>.exe` from a
   PowerShell and the launcher attaches to it. Screenshots with Win+Shift+S.
   With the example, the step and what showed instead, the report is complete.

### The PushinShop PDV on Windows

1. Clone `pam-desktop-pdv` and `pushinshop-pdv` side by side (the PDV finds
   the framework through a Composer path repository `../pam-desktop-pdv`).
   Symbolic links need Developer Mode on Windows; without it, set
   `"symlink": false` in the PDV's `composer.json` before `composer install`.
2. In `pushinshop-pdv`: `composer install`, then
   `..\pam-desktop-pdv\target\debug\pam-desktop.exe dev .`.
3. Check, besides the list above: the login (typing into e-mail and password
   enables the button, the eye shows the password), the icons of the fields
   (PrimeIcons, an `asset://` font), the logo (an SVG), POS, tables (scrolling of the main area
   and the sidebar; nothing cut on the right), the product editor (category
   popup with search), wallet, and printing a receipt to "Microsoft Print to PDF".
4. `pam-desktop.exe build . --format directory --format portable` and start
   `bin\pdv.exe` from the copied directory. The build lists the PHP
   extensions it shipped (`mbstring`, `intl`, `gd`, `sodium`, `openssl` for
   the PDV today).

A bundle of the PDV made under Wine is at
`/media/davidbalbino/BACKUP/pdv-windows/pdv-0.1.0-windows-x86_64-wine-built.zip`
(GNU host, `WebView2Loader.dll` beside it, PHP 8.5.12 with the
`vcruntime140` DLLs of the Python 3.14.10 embeddable archive, both checked
against their published SHA-256; `msvcp140.dll` was left out, so it comes
from the system's redistributable). It is for a first look only: the
reference bundle is the one `pam-desktop build` makes on Windows.

### What was checked before the first run

Not on Windows. Cross-compilation (`x86_64-pc-windows-gnu`, Ubuntu 24.04
MinGW 13) of the host and the backend with Clippy; the backend's unit tests
and the examples under Wine 9.0 with Xvfb. Wine is not Windows: it says that
the code starts, paints and answers, not how Windows draws it.

- **Wine ran:** `counter` (clicks reach PHP and the count updates),
  `controls` (typing into the masked phone field reaches PHP, placeholders
  dimmed, no focus ring after a click), `text` (inline `kbd` boxes take their
  width in the line and paint their background), `pam-desktop build` of the
  PDV, and the PDV bundle up to its login screen, with the icon font.
- **Wine could not show:** borders (Wine's Direct2D does not fill the
  border rings), hit-testing of text (Wine's DirectWrite has no
  `HitTestTextPosition`, `HitTestTextRange`, `HitTestPoint`: selection and
  pressable spans do nothing), HTTPS from the host (Wine's certificate chain
  check fails), the `curl` extension (does not start), PowerShell (no launcher
  icon), WebView2, Media Foundation, toasts, SVG (Wine's Direct2D has no SVG
  renderer; the image fails with a message). PHP reads no file through a
  drive mapped to a Unix path (`Z:`), so applications ran from `C:`.

- **Fixed before the first run** (the classes of problem the first macOS run
  found, and what Wine showed):
  - Packaged fonts (`asset://…ttf`) were not loaded: DirectWrite now reads
    them from the asset root into a collection of their own (and GDI
    registers them privately for the `EDIT` controls).
  - Paths given to PHP were verbatim (`\\?\C:\…`, from `canonicalize`):
    PHP's `realpath`, `chdir` and includes fail on them, so the worker could
    not start, and the build's search for the extensions an application uses
    read nothing and left `mbstring` out of the PDV. They are plain paths now.
  - The bundle took every DLL beside PHP; it now takes those PHP and the
    shipped extensions load, plus the Visual C++ runtime (app-local), and the
    build stops with a clear message when PHP cannot start or the host needs
    a `WebView2Loader.dll` that is not beside it.
  - A failing PowerShell icon render no longer fails the whole build.
  - SVG images are decoded (Direct2D SVG, rasterized at the shown size on the
    decoding worker); the unit test draws a two-colour SVG and checks the
    pixels are upright on Windows.
  - HTTPS from PHP: a CA bundle of the Windows store and Mozilla's roots is
    given to curl and OpenSSL. Under Wine, `file_get_contents` of an HTTPS
    URL worked through it and a self-signed certificate was refused.
  - Scroll bars: `scrollbar-width` (`none`, `thin`, a length) and
    `scrollbar-color` are drawn; a styled bar in a reserved gutter stays
    visible. The bars were already overlays drawn by the backend, so they
    never take room from content (the macOS legacy scroller problem cannot
    happen).
  - Line breaking follows CSS (`overflow-wrap`, `word-break: break-all`),
    `white-space: nowrap` clips without an ellipsis, and measured heights stay
    fractional, like GTK. `hyphens` has no DirectWrite equivalent.
  - Inline boxes of nested texts (`TextSpanBoxes`, a `<kbd>`): their edges are
    DirectWrite inline objects of the padding plus border width, which do not
    change the line height (unless the box has a line height of its own), and
    the box is painted per line under the text; selection and hit-testing skip
    the edges. A head or middle ellipsis is not cut around boxes (the end one is).
  - Already right in the code, checked against the macOS findings: the
    `EDIT` controls report every change (`EN_CHANGE` through `WM_COMMAND`),
    the focus ring follows `:focus-visible` (keyboard only), a placeholder
    without a colour is the text colour at 45 % over the field, and
    `backdropBlur` is implemented (item 17 below).

### What to check first, by risk

1. **Child windows over the swap chain.** Text inputs (`EDIT`) and the WebView2 container are GDI child windows over a flip-model swap chain with `WS_CLIPCHILDREN`: confirm that they show and do not flicker while scrolling and resizing (`examples/controls`, `examples/webview`).
2. **Media Foundation** (`examples/media`): the shared D3D11 device with multithread protection, `OnVideoStreamTick`, `TransferVideoFrame` into a BGRA texture, a raw file path in `SetSource` (spaces, `#`, non-ASCII), a machine without video support.
3. **DirectWrite**: multi-line ellipsis, the baseline with a fixed `lineHeight`, measurement equal to drawing.
4. **Direct2D**: the nine-patch shadow, the eraser of `DrawingCanvas`, gradients.
5. **Presentation**: `Present1` with dirty rectangles (turn off with `PAM_DESKTOP_FULL_PRESENT=1`) and recovery after a lost device (`dxcap -forcetdr`).
6. **WebView2**: environment, controller, bridge script, navigation; the user data folder `%LOCALAPPDATA%\<applicationId>\WebView2`.
7. **UI Automation**: check with Inspect and Narrator.
8. **`EDIT` controls**: the placeholder, password fields, the wheel inside a text area, IME.
9. **Custom frame** (`windowDrag`, undecorated windows), full screen from maximized, a DPI change between monitors.
10. **First frame** painted before the window shows; autofocus on first show.
11. **Tray** (`NIN_SELECT`) and toast notifications without a Start menu shortcut.
12. **Single instance**: the second launch finds the first through a message-only window and hands the link with `WM_COPYDATA`.
13. **Console**: a release host is a windowed program. `pam-desktop build` from a terminal must still print (the host attaches to the parent console), and no console window may open beside an application or its PHP worker.
14. **Share** (`examples/modules`, the Share button): files are resolved before the dialog shows, waiting up to 3 seconds on the UI thread; if that wait never completes, every share with files fails. Check text only, files only (a path with spaces and non-ASCII), both, two shares in a row, and closing the application with the dialog open.
15. **Head and middle ellipsis** (`ellipsize()` with `numberOfLines(1)`): DirectWrite only trims the end of a line, so the backend shortens the text itself, measuring candidates until one fits. Check a one-line text narrower than its content in both modes: one ellipsis, at the start or in the middle, and the text reaching the right edge without being clipped. Then resize the window slowly (the cut must move without flicker and the ellipsis must go away when the text fits), try runs of different styles (the ellipsis takes the style of the run next to it), emoji and combining marks next to the cut (no broken glyph), and a pressable span or a selection after the cut (they must answer for the characters shown).
16. **Dragging the seek bar** (`examples/media`, the controls drawn over a player): a press on the track seeks as before and now holds the bar until the button is released. Check that the white part of the bar and the elapsed time follow the pointer at once, also when the pointer leaves the player or the window with the button down; that the picture catches up during a slow drag and lands on the release position after a fast one (moves only seek when `IMFMediaEngine::IsSeeking` says the previous seek is done; if it never does, only the release seeks); that a plain click still seeks once; that the controls hide again after a release outside the player; and that Alt+Tab in the middle of a drag ends it without the bar following the pointer afterwards.
17. **`backdropBlur`** (a translucent box with `backdropBlur` over an image or a scrolling list): the backend flushes the frame, copies the pixels under the box from the back buffer (`CopyFromBitmap` from the swap chain's bitmap, in the middle of `BeginDraw`), blurs the copy with the Gaussian blur effect and draws it back clipped to the box. Check first that the window still paints at all with such a box on screen; a frame that fails turns the blur off for the rest of the run and is drawn again, and `PAM_DESKTOP_BACKDROP_BLUR=0` turns it off from the start. Then check that the blur shows what is really behind the box, in the right place at 100% and at 150% scaling, with rounded corners, with no dark or transparent rim at the edges of the box or of the window, following the content while it scrolls or a video plays behind it, and fading with the box when its opacity animates. Watch memory while resizing a window with a large blurred box.
18. **Dark menus** (`examples/shell` or any application with a menu bar, with the system in dark mode and with `Appearance` forced to dark on a light system): popup menus go dark through three functions that uxtheme exports by ordinal only (133, 135, 136), called on build 17763 and later; the menu bar is drawn by the window from the undocumented messages `0x0091` and `0x0092`, and the light line under it is painted over after `WM_NCPAINT` and `WM_NCACTIVATE`. Check that the application still starts in dark mode (a wrong ordinal signature would crash at the first window); that the bar, its items (normal, hovered, open, disabled, with and without the Alt underlines), the menus that drop from it, the context menus and the tray menu are dark; that no light line stays under the bar after activating, deactivating, resizing and maximizing the window; that switching the system or the application between light and dark while it runs changes all of them both ways; and that in light mode everything looks exactly as before. Also open a file dialog and an alert after menus went dark, to see what else the application mode changed.
19. **The icon of the launcher** (`pam-desktop build` of an application with a PNG `icon`): the build renders the icon at 16, 32, 48 and 256 pixels with PowerShell and writes them into `bin\<name>.exe` as `RT_ICON` resources 1 to 4 and `RT_GROUP_ICON` 1 (`BeginUpdateResourceW`, `UpdateResourceW`, `EndUpdateResourceW`), before anything is signed. Check that the build does not fail at that step (an antivirus scanning the fresh copy can make `EndUpdateResourceW` fail); that Explorer shows the application's icon for the launcher in the small, medium and extra large views, and the window and the taskbar show it too; that the launcher still starts, with its manifest intact (crisp at 150% scaling, themed controls); that `--sign` still signs it and the signature verifies; and what happens when the host itself was signed before it was copied.

20. **PHP on first run** (a machine without PHP): `pam-desktop dev` downloads the official PHP 8.5 NTS x64 archive from windows.php.net, checks its SHA-256 and installs it under `%LOCALAPPDATA%\PAM Desktop\php\<version>`. Check the download behind a proxy, a machine without the Visual C++ Redistributable (the host must say so and name the download), a second run that reuses the installed PHP, and `pam-desktop build` bundling only the DLLs PHP imports.
21. **Packaged fonts** (`asset://…ttf` in `font-family`, icon fonts): they are loaded into a DirectWrite collection and, for `EDIT` controls, as private GDI fonts. Check that icons are not missing-glyph boxes, in text and in inputs.
22. **Inline boxes** (`examples/text`, a `<Text>` with a nested `<Text class="kbd">`): DirectWrite inline objects reserve the box edges. Check that the words around a box move, that a box that wraps paints one piece per line, and that clicking its padding places the caret at its edge.
23. **Scrollbars** (`scrollbar-gutter: stable`, `scrollbar-width: thin | none | 8px`, `scrollbar-color`): the content keeps its width when the bar appears; `none` hides the bar; a reserved gutter shows a classic bar you can drag.
24. **Key capture** (`captureKeys`): captured keys are taken before `TranslateMessage`. Check that Space on a focused element with `captureKeys="Space"` reaches `on:keyDown` and types nothing, and that Control shortcuts still work.
25. **Printing** (`pushinbr/pam-desktop-printing`): the system print dialog, "Microsoft Print to PDF", and a cancelled dialog reported as cancelled; the UI must stay responsive while a slow driver spools.
26. **Typed drag and drop** (`DragSource`, `DropTarget`): enter, leave and drop on a target that accepts the type, nothing on one that does not, auto-scroll near the edge of a list, and a drag inside a secondary window.
27. **SVG images** (`examples/images`, the PDV logo): drawn by Direct2D's SVG renderer into a WIC bitmap on the decoding worker. Check that one shows, sharp at the size of its box, not upside down, and that a failing one reports `ImageFailed` with a message; `cargo test -p pam-desktop-win32` draws one and checks its pixels.
28. **HTTPS from PHP**: with the PHP PAM Desktop installed and in a bundle, run `file_get_contents('https://example.com')` and a `curl_exec` from the application (`PAM_DESKTOP_DEBUG=1`), behind a corporate proxy with its own root if there is one; then remove the network and check the error is a PHP warning, not a hang. The bundle is `%LOCALAPPDATA%\<applicationId>\Cache\php-ca-bundle.pem` (or the development cache directory).

### Known differences from the GTK backend

Text and inputs:

- `Head` and `Middle` ellipsis apply to a text limited to one line. With more lines the ellipsis is at the end.
- `hyphens: auto` is not applied. Around an inline box only an end ellipsis is cut.
- Inputs are `EDIT` controls, also the multi-line one. `letterSpacing`, `selectionColor`, `caretColor`, `spellcheck` and `capitalization` have no equivalent; undo has one level.
- The background of an input is the nearest opaque color behind it. A gradient or translucency behind the text does not show.
- Child windows are always above the scene: an input is hidden while an overlay covers it, so its text disappears under a translucent modal.

Painting, pointer and media:

- `backdropBlur` blurs a copy of what the window has painted behind the view. It is left out where that copy cannot be taken: inside a rounded or rotated clipping box (`overflow: hidden`), inside a translucent ancestor, on a view that also has `blur` or a color matrix, and in the image that follows the pointer during a drag. Text inputs and web views are child windows and are not part of what is blurred.
- `Magnify` and `Rotate` come from touch only; a touchpad pinch arrives as Control plus wheel, which the core treats as a pinch.
- SVG images are drawn by Direct2D's SVG renderer (Windows 10 1703 or later) at the size they are shown at, or at twice their natural size when that is not known, like GTK. It implements SVG 1.1 static content without text (`<text>` is not drawn), scripts, animation or external references; a `.svgz` (compressed) file is not read.
- `hyphens: auto` has no effect: DirectWrite does not hyphenate. Words are not split (with `overflow-wrap` or `word-break` they break without a hyphen); a soft hyphen (U+00AD) in the text is still a place where the line may break.
- `curl`, `file_get_contents('https://…')` and the other PHP streams trust a CA bundle the host writes at every start to the application's cache directory (`php-ca-bundle.pem`): the roots of the Windows store, enterprise roots included, and Mozilla's roots that ship with PAM Desktop. It is given to the PHP PAM Desktop installed and to the PHP of a bundle (`-d curl.cainfo=… -d openssl.cafile=…`, also in `PAM_DESKTOP_PHP_CA_FILE`); a PHP chosen with `PAM_DESKTOP_PHP` keeps its own `php.ini`.

Web view and accessibility:

- `domStorage` is not configurable; `inlineMedia` is decided by the first page of the process.
- Application shortcuts do not fire while a page has the focus.
- Custom accessibility actions are not exposed.

Shell:

- An undecorated window shows no menu bar (accelerators still work).
- `destructive` menu items look like the others.
- Menus follow dark mode through undocumented parts of the theme engine, from Windows 10 1809 on. Between 1809 and 1903 popup menus are dark only when the system is, also when the application chose the dark appearance itself. Once menus were dark, the system's file dialogs opened by the application may be dark too.
- Notifications: the host registers the `applicationId` as the AppUserModelID under `HKCU\Software\Classes\AppUserModelId`; when a toast cannot be shown, a balloon is.
- Only copy drags; the drag image hides over the own window.
- `Share::text()` and `Share::files()` open the share dialog of the window; the call reports success when the dialog was asked to open, also when the user cancels it.

## What was left out

- An application icon in the Windows launcher when the application icon is an SVG, or when there is none: the launcher is a copy of the host, its icon is made from a PNG, and nothing in the build renders an SVG on Windows. The build says so in a note.

## Reporting what you find

The most useful report is the example, the step and what showed instead: a screenshot for anything visual, the panic message and backtrace for a crash (`RUST_BACKTRACE=1`), and the output of `PAM_DESKTOP_DEBUG=1` when an event did not reach PHP.
