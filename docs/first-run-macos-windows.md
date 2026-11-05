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
- `blur`, `backdropBlur` and `colorMatrix` use Core Image filters.
- A conic gradient is a fan of 360 slices.

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

### Known differences from the GTK backend

Text and inputs:

- `Head` and `Middle` ellipsis apply to a text limited to one line. With more lines the ellipsis is at the end.
- Inputs are `EDIT` controls, also the multi-line one. `letterSpacing`, `selectionColor`, `caretColor`, `spellcheck` and `capitalization` have no equivalent; undo has one level.
- The background of an input is the nearest opaque color behind it. A gradient or translucency behind the text does not show.
- Child windows are always above the scene: an input is hidden while an overlay covers it, so its text disappears under a translucent modal.

Painting, pointer and media:

- `backdropBlur` blurs a copy of what the window has painted behind the view. It is left out where that copy cannot be taken: inside a rounded or rotated clipping box (`overflow: hidden`), inside a translucent ancestor, on a view that also has `blur` or a color matrix, and in the image that follows the pointer during a drag. Text inputs and web views are child windows and are not part of what is blurred.
- `Magnify` and `Rotate` come from touch only; a touchpad pinch arrives as Control plus wheel, which the core treats as a pinch.

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

- An application icon in the executable on Windows: the launcher of a bundle is a copy of the host, with the host's resources.

## Reporting what you find

The most useful report is the example, the step and what showed instead: a screenshot for anything visual, the panic message and backtrace for a crash (`RUST_BACKTRACE=1`), and the output of `PAM_DESKTOP_DEBUG=1` when an event did not reach PHP.
