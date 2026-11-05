# First run on macOS and Windows

The AppKit and Win32 backends were written on Linux and type-checked by cross compilation. Neither has run on a real machine. This page is the order in which to look at things on the first run, the behavior that is known to differ from the GTK backend, and what was left out.

Linux is the reference: every example under `examples/` was run and checked there.

## Before anything else

1. Build and run in **debug** first: `cargo build -p pam-desktop-host`, then `target/debug/pam-desktop dev examples/counter`.
   - macOS: objc2 checks the signature of every Objective-C method the backend overrides when its classes are registered. A mismatch is a panic with the name of the method, at startup.
   - Windows: the backend keeps its state in a `RefCell`. A re-entrant borrow is ignored in release and is a `debug_assert` in debug.
2. Then walk through the examples in this order, which goes from the least to the most platform code: `counter`, `controls`, `lists`, `text`, `images`, `motion`, `navigation`, `shell`, `windows`, `dragdrop`, `canvas`, `media`, `webview`, `links`, `showcase`.
3. `PAM_DESKTOP_DEBUG=1` prints every event the host sends to PHP.

## macOS (`pam-desktop-appkit`)

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
- `blur`, `backdropBlur` and `colorMatrix` use Core Image filters.
- A conic gradient is a fan of 360 slices.

Images, media and web:

- EXIF orientation is applied; SVG is not decoded.
- Media `Center` and `Repeat` fits are shown as aspect fit.
- `domStorage: false` uses a non-persistent data store; changing `domStorage` or `inlineMedia` recreates the view and reloads the page.

System:

- Notifications need a bundle identifier: a loose binary posts none.
- `Window::progress()` is a Dock badge ("42%").
- A modal window with a parent is a sheet.
- Links arrive through `application:openURLs:`, also the one that launched the application, so `Linking::listen()` receives it and `Linking::initial()` may be empty.
- Battery, network and Low Power Mode are read with `pmset`, `route` and `networksetup`, at most every 10 seconds.
- `Screen::secure()` is not implemented yet (the window sharing type).

## Windows (`pam-desktop-win32`)

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

### Known differences from the GTK backend

Text and inputs:

- `Head` and `Middle` ellipsis become an ellipsis at the end.
- Inputs are `EDIT` controls, also the multi-line one. `letterSpacing`, `selectionColor`, `caretColor`, `spellcheck` and `capitalization` have no equivalent; undo has one level.
- The background of an input is the nearest opaque color behind it. A gradient or translucency behind the text does not show.
- Child windows are always above the scene: an input is hidden while an overlay covers it, so its text disappears under a translucent modal.

Painting, pointer and media:

- `backdropBlur` is not implemented.
- `Magnify` and `Rotate` come from touch only; a touchpad pinch arrives as Control plus wheel, which the core treats as a pinch.
- The seek bar of the drawn media controls takes clicks, not drags.

Web view and accessibility:

- `domStorage` is not configurable; `inlineMedia` is decided by the first page of the process.
- Application shortcuts do not fire while a page has the focus.
- Custom accessibility actions are not exposed.

Shell:

- An undecorated window shows no menu bar (accelerators still work).
- `destructive` menu items look like the others; menus do not follow dark mode.
- Notifications: the host registers the `applicationId` as the AppUserModelID under `HKCU\Software\Classes\AppUserModelId`; when a toast cannot be shown, a balloon is.
- Only copy drags; the drag image hides over the own window.
- Text scale and reduced motion are read at startup.

## What neither backend got

- `Share::text()` and `Share::files()` report that sharing is not available, as on Linux.
- An application icon in the executable on Windows: the launcher of a bundle is a copy of the host, with the host's resources.

## Reporting what you find

The most useful report is the example, the step and what showed instead: a screenshot for anything visual, the panic message and backtrace for a crash (`RUST_BACKTRACE=1`), and the output of `PAM_DESKTOP_DEBUG=1` when an event did not reach PHP.
