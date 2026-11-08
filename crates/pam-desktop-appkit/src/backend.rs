//! `Backend` implementation on AppKit.

use std::collections::HashMap;
use std::process::ExitCode;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSDraggingItem, NSEvent, NSEventModifierFlags, NSEventType,
    NSImage, NSView, NSWindow, NSWindowButton, NSWindowOrderingMode, NSWindowSharingType,
    NSWindowStyleMask, NSWindowTabbingMode, NSWindowTitleVisibility,
};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSArray, NSURL};
use pam_desktop_ui::accessibility::Accessibility;
use pam_desktop_ui::backend::{
    AlertConfig, Backend, BackendEvent, DrawCommand, ImageContent, ImageKey, ImageSource,
    InputConfig, MAIN_WINDOW, MediaState, NodeId, ScrollConfig, TextContent, ViewClass,
    WebViewState, WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::scene::IdMap;
use pam_desktop_ui::shell::{
    Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand,
    WindowState,
};
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};

use crate::accessibility::{self, Applied};
use crate::clock::{FrameClock, WakeTimer};
use crate::events::{Events, MainThread};
use crate::image::ImageStore;
use crate::input::InputHost;
use crate::media::MediaHost;
use crate::scroll::ScrollHost;
use crate::shell::{self, Shell};
use crate::text;
use crate::util::{cg_size, ns_string};
use crate::view::{self, ImageState, NodeView};
use crate::webview::WebHost;
use crate::window::{self, HostView, PamWindow, WindowDelegate};

struct ViewEntry {
    /// The node's view, its shadows and its clip.
    node: NodeView,
    /// The viewport of a scroll view; children live in its document.
    scroll: Option<ScrollHost>,
    /// The native text control of input classes.
    input: Option<InputHost>,
    /// The player of a media view.
    media: Option<MediaHost>,
    /// The page of a web view.
    web: Option<WebHost>,
    /// What assistive technology was told, when it is more than the defaults.
    accessibility: Option<Box<Accessibility>>,
    /// The text the view shows, its accessible name unless a label was given.
    plain_text: Option<String>,
    applied: Applied,
}

impl ViewEntry {
    /// Native views of the node itself, below its children.
    fn native_views(&self, into: &mut Vec<Retained<NSView>>) {
        into.extend(self.scroll.as_ref().map(ScrollHost::view));
        into.extend(self.input.as_ref().map(InputHost::view));
        into.extend(self.media.as_ref().and_then(MediaHost::view));
        into.extend(self.web.as_ref().and_then(WebHost::view));
    }

    fn apply_accessibility(&mut self, events: &Rc<Events>) {
        let control = self.input.as_ref().map(InputHost::focus_target);
        accessibility::apply(
            &self.node.view,
            control.as_deref(),
            self.node.view.node_id(),
            events,
            self.accessibility.as_deref(),
            self.plain_text.as_deref(),
            &mut self.applied,
        );
    }
}

struct WindowEntry {
    window: Retained<PamWindow>,
    /// Kept alive: a window references its delegate weakly.
    _delegate: Retained<WindowDelegate>,
    /// The content view: parent of the root node and the overlays.
    host: Retained<HostView>,
    presented: bool,
    root: Option<NodeId>,
    overlays: Vec<NodeId>,
    /// The window this one belongs to, and whether it blocks it.
    parent: Option<WindowId>,
    modal: bool,
}

pub struct AppKitBackend {
    mtm: MainThreadMarker,
    events: Rc<Events>,
    views: IdMap<ViewEntry>,
    windows: HashMap<WindowId, WindowEntry>,
    images: Rc<ImageStore>,
    frames: FrameClock,
    wake: WakeTimer,
    shell: Shell,
    /// The appearance the application forces, if any.
    forced_dark: Option<bool>,
    /// Windows are kept out of screenshots and recordings.
    content_protected: bool,
    /// Watches the display options of the system (reduce motion).
    display_options: Option<shell::Registration>,
}

impl AppKitBackend {
    #[must_use]
    pub fn new(mtm: MainThreadMarker, events: Rc<Events>) -> Self {
        let mut shell = Shell::new(mtm, Rc::clone(&events));
        // Until the application defines its menu bar, the standard menus
        // give it Quit and the editing shortcuts of text fields.
        shell.set_menu_bar(&[]);
        let display_options = shell::watch_display_options(&events);
        Self {
            mtm,
            images: ImageStore::install(&events),
            frames: FrameClock::new(mtm, &events),
            wake: WakeTimer::default(),
            shell,
            events,
            views: IdMap::default(),
            windows: HashMap::new(),
            forced_dark: None,
            content_protected: false,
            display_options: Some(display_options),
        }
    }

    fn native_window(&self, window: WindowId) -> Option<Retained<NSWindow>> {
        self.windows
            .get(&window)
            .map(|entry| Retained::into_super(entry.window.clone()))
    }

    /// The window host holds the root view and, above it, the overlays.
    fn arrange_window(entry: &WindowEntry, views: &IdMap<ViewEntry>) {
        let mut wanted = Vec::new();
        // The root may also be an overlay (a modal that is the whole
        // application): it is stacked once, where the overlays go.
        let root = entry.root.filter(|root| !entry.overlays.contains(root));
        for id in root.iter().chain(entry.overlays.iter()) {
            if let Some(view) = views.get(id) {
                view.node.stack(&mut wanted);
            }
        }
        view::arrange(&entry.host, &wanted);
    }

    /// Shows a window for the first time: on its own, attached to its
    /// parent, or as a sheet that blocks it.
    fn present(&self, window: WindowId) {
        let Some(entry) = self.windows.get(&window) else {
            return;
        };
        let parent = entry
            .parent
            .and_then(|parent| self.windows.get(&parent))
            .map(|parent| parent.window.clone());
        match parent {
            Some(parent) if entry.modal => parent.beginSheet_completionHandler(&entry.window, None),
            Some(parent) => {
                // SAFETY: both windows are alive; the child is detached in
                // `close_window` before either goes away.
                unsafe {
                    parent.addChildWindow_ordered(&entry.window, NSWindowOrderingMode::Above)
                };
                entry.window.makeKeyAndOrderFront(None);
            }
            None => {
                entry.window.makeKeyAndOrderFront(None);
                #[expect(deprecated, reason = "the replacement needs macOS 14")]
                NSApplication::sharedApplication(self.mtm).activateIgnoringOtherApps(true);
            }
        }
    }

    /// Stops what would outlive the windows: the pending wake-up, the
    /// status item, the shortcut monitor and the decoding threads.
    fn release_services(&mut self) {
        self.wake.cancel();
        if let Some(registration) = self.display_options.take() {
            shell::unwatch_display_options(&registration);
        }
        self.shell.shutdown();
        self.images.uninstall();
    }

    /// Where the children of a node go, and the node's own native views
    /// that stay below them.
    fn container(
        &mut self,
        id: NodeId,
        has_children: bool,
    ) -> Option<(Retained<NSView>, Vec<Retained<NSView>>)> {
        let entry = self.views.get_mut(&id)?;
        if let Some(scroll) = &entry.scroll {
            return Some((Retained::into_super(scroll.document.clone()), Vec::new()));
        }
        // A clipping node gets its inner view with its first child.
        let container = if has_children {
            entry.node.content_parent()
        } else {
            entry.node.content_view()
        };
        let mut native = Vec::new();
        entry.native_views(&mut native);
        Some((container, native))
    }
}

impl Backend for AppKitBackend {
    fn set_asset_root(&mut self, root: &std::path::Path) {
        crate::fonts::set_root(root);
    }

    fn open_window(&mut self, window: WindowId, config: &WindowConfig) {
        if let Some(entry) = self.windows.get(&window) {
            entry.window.setTitle(&ns_string(&config.title));
            shell::window_command(
                self.mtm,
                &entry.window,
                &WindowCommand::SetResizable(config.resizable),
            );
            // A new size from the application resizes a secondary window.
            if window != MAIN_WINDOW && window::content_metrics(&entry.window).0 != config.size {
                entry.window.setContentSize(cg_size(config.size));
            }
            return;
        }
        let mut style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable;
        if config.resizable {
            style |= NSWindowStyleMask::Resizable;
        }
        if !config.decorated {
            // The application draws its own chrome: the content covers the
            // whole window, under a title bar that shows nothing.
            style |= NSWindowStyleMask::FullSizeContentView;
        }
        let content = CGRect::new(CGPoint::ZERO, cg_size(config.size));
        let native = PamWindow::new(self.mtm, window, &self.events, content, style);
        native.setTitle(&ns_string(&config.title));
        if !config.decorated {
            native.setTitlebarAppearsTransparent(true);
            native.setTitleVisibility(NSWindowTitleVisibility::Hidden);
            for button in [
                NSWindowButton::CloseButton,
                NSWindowButton::MiniaturizeButton,
                NSWindowButton::ZoomButton,
            ] {
                if let Some(button) = native.standardWindowButton(button) {
                    button.setHidden(true);
                }
            }
        }
        if let Some(minimum) = config.min_size {
            native.setContentMinSize(cg_size(minimum));
        }
        if self.content_protected {
            native.setSharingType(NSWindowSharingType::None);
        }
        // One window is one window: no automatic tabs.
        native.setTabbingMode(NSWindowTabbingMode::Disallowed);
        // Focusable views come and go: AppKit keeps the Tab order.
        native.setAutorecalculatesKeyViewLoop(true);
        let host = HostView::new(self.mtm, window, &self.events);
        native.setContentView(Some(&host));
        native.center();
        let delegate = WindowDelegate::new(self.mtm, window, &self.events, &native);
        native.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        self.windows.insert(
            window,
            WindowEntry {
                window: native,
                _delegate: delegate,
                host,
                presented: false,
                root: None,
                overlays: Vec::new(),
                parent: config.parent.filter(|parent| *parent != window),
                modal: config.modal,
            },
        );
    }

    fn close_window(&mut self, window: WindowId) {
        let Some(entry) = self.windows.remove(&window) else {
            return;
        };
        if let Some(parent) = entry.parent.and_then(|parent| self.windows.get(&parent)) {
            if entry.presented && entry.modal {
                parent.window.endSheet(&entry.window);
            } else if entry.presented {
                parent.window.removeChildWindow(&entry.window);
            }
        }
        // The display link belongs to a view of some window: a pending
        // frame is asked again from one that is still open.
        let pending = self.frames.is_requested();
        self.frames.reset();
        // Nothing of the window keeps keyboard focus: AppKit may move the
        // first responder again while it tears the window down, when the
        // focus the window tracked is already gone.
        entry.window.makeFirstResponder(None);
        entry.window.setDelegate(None);
        entry.window.close();
        // Released on the next turn of the run loop: the window may be
        // closing from inside one of its own callbacks (an event it sent
        // drained the runtime).
        self.events.defer(move || drop(entry));
        if pending {
            self.request_frame();
        }
    }

    fn window_metrics(&self, window: WindowId) -> WindowMetrics {
        let Some(entry) = self.windows.get(&window) else {
            return WindowMetrics::default();
        };
        let (size, scale) = window::content_metrics(&entry.window);
        let refresh_rate = entry
            .window
            .screen()
            // The refresh rate of a screen is public since macOS 12.
            .filter(|screen| screen.respondsToSelector(sel!(maximumFramesPerSecond)))
            .map(|screen| screen.maximumFramesPerSecond() as f32)
            .filter(|rate| *rate > 1.0)
            .unwrap_or(60.0);
        WindowMetrics {
            size,
            scale,
            // macOS has no system-wide text size: applications scale themselves.
            font_scale: 1.0,
            dark: shell::system_is_dark(self.mtm, self.forced_dark.is_some()),
            refresh_rate,
            reduced_motion: shell::reduced_motion(),
        }
    }

    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.root = root;
        Self::arrange_window(entry, &self.views);
        if root.is_some() && !entry.presented {
            entry.presented = true;
            self.present(window);
        }
    }

    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]) {
        let Some(entry) = self.windows.get_mut(&window) else {
            return;
        };
        entry.overlays = overlays.to_vec();
        Self::arrange_window(entry, &self.views);
        // A boot failure can produce host chrome before any application root.
        if !overlays.is_empty() && !entry.presented {
            entry.presented = true;
            self.present(window);
        }
    }

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        let (mtm, events) = (self.mtm, &self.events);
        let node = NodeView::new(mtm, id, events);
        // A new node has no clip yet: its native views go straight into it.
        let parent = node.content_view();
        let scroll =
            (class == ViewClass::Scroll).then(|| ScrollHost::new(mtm, id, events, &parent));
        let input = match class {
            ViewClass::Input => Some(InputHost::new(mtm, id, false, &node.view, &parent, events)),
            ViewClass::TextArea => Some(InputHost::new(mtm, id, true, &node.view, &parent, events)),
            _ => None,
        };
        let media = (class == ViewClass::Media).then(|| MediaHost::new(mtm, id, events));
        let web = (class == ViewClass::WebView).then(|| WebHost::new(mtm, id, events));
        self.views.insert(
            id,
            ViewEntry {
                node,
                scroll,
                input,
                media,
                web,
                accessibility: None,
                plain_text: None,
                applied: Applied::default(),
            },
        );
    }

    fn destroy_view(&mut self, id: NodeId) {
        let Some(mut entry) = self.views.remove(&id) else {
            return;
        };
        if let Some(input) = &entry.input {
            input.destroy();
        }
        if let Some(media) = &mut entry.media {
            media.destroy();
        }
        if let Some(web) = &mut entry.web {
            web.destroy();
        }
        if let Some(scroll) = &entry.scroll {
            scroll.destroy();
        }
        entry.node.detach();
        // Released on the next turn of the run loop: a native view may be
        // going away from inside one of its own callbacks (a scroll or a
        // focus change it reported drained the runtime).
        self.events.defer(move || drop(entry));
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        let Some((container, mut wanted)) = self.container(parent, !children.is_empty()) else {
            return;
        };
        for child in children {
            if let Some(entry) = self.views.get(child) {
                entry.node.stack(&mut wanted);
            }
        }
        view::arrange(&container, &wanted);
    }

    fn set_frame(&mut self, id: NodeId, frame: Rect) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        let resized = entry.node.frame().size() != frame.size();
        entry.node.set_frame(frame);
        if !resized {
            return;
        }
        let size = frame.size();
        if let Some(scroll) = &mut entry.scroll {
            scroll.set_viewport(size);
        }
        if let Some(input) = &mut entry.input {
            input.layout(size);
        }
        if let Some(media) = &mut entry.media {
            media.layout(size);
        }
        if let Some(web) = &mut entry.web {
            web.layout(size);
        }
    }

    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration) {
        if let Some(entry) = self.views.get_mut(&id) {
            entry.node.set_decoration(decoration);
        }
    }

    fn set_layer(&mut self, id: NodeId, layer: &LayerState) {
        if let Some(entry) = self.views.get_mut(&id) {
            entry.node.set_layer(layer);
        }
    }

    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        entry.node.view.set_text(text);
        // The text is the accessible name unless a label was given.
        entry.plain_text = text.map(|content| content.block.plain_text());
        entry.apply_accessibility(&self.events);
    }

    fn set_key_capture(&mut self, id: NodeId, keys: &str) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.set_key_capture(keys);
        }
    }

    fn set_pointer_events(&mut self, id: NodeId, target_self: bool, target_children: bool) {
        if let Some(entry) = self.views.get(&id) {
            entry
                .node
                .view
                .set_pointer_events(target_self, target_children);
        }
    }

    fn set_focusable(&mut self, id: NodeId, focusable: bool) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.set_focusable(focusable);
        }
    }

    fn set_accessibility(&mut self, id: NodeId, accessibility: Option<&Accessibility>) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        entry.accessibility = accessibility.cloned().map(Box::new);
        entry.apply_accessibility(&self.events);
    }

    fn focus(&mut self, id: Option<NodeId>) {
        match id.and_then(|id| self.views.get(&id)) {
            Some(entry) => match &entry.input {
                Some(input) => input.focus(),
                None => {
                    if let Some(window) = entry.node.view.window() {
                        window.makeFirstResponder(Some(&entry.node.view));
                    }
                }
            },
            None => {
                for entry in self.windows.values() {
                    entry.window.makeFirstResponder(None);
                }
            }
        }
    }

    fn set_input_config(&mut self, id: NodeId, config: &InputConfig) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        let parent = entry.node.content_parent();
        if let Some(input) = &mut entry.input {
            input.configure(config, &entry.node.view, &parent);
        }
    }

    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
        if let Some(input) = self.views.get(&id).and_then(|entry| entry.input.as_ref()) {
            input.set_text(text, selection);
        }
    }

    fn set_input_selection(&mut self, id: NodeId, start: u32, end: u32) {
        if let Some(input) = self.views.get(&id).and_then(|entry| entry.input.as_ref()) {
            input.set_selection(start, end);
        }
    }

    fn load_image(&mut self, key: ImageKey, source: ImageSource, max_dimension: Option<u32>) {
        self.images.load(key, source, max_dimension);
    }

    fn release_image(&mut self, key: ImageKey) {
        self.images.release(key);
    }

    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>) {
        let Some(entry) = self.views.get(&id) else {
            return;
        };
        let state = image.and_then(|content| {
            Some(ImageState {
                content: *content,
                image: self.images.get(content.key)?,
            })
        });
        entry.node.view.set_image(state);
    }

    fn text_index_at(&self, id: NodeId, point: Point) -> Option<u32> {
        self.views.get(&id)?.node.view.text_index_at(point)
    }

    fn set_text_selection(&mut self, id: NodeId, range: Option<(u32, u32)>) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.set_text_selection(range);
        }
    }

    fn print_document(
        &mut self,
        _window: WindowId,
        document: pam_desktop_ui::printing::Document,
        complete: pam_desktop_ui::printing::PrintCompletion,
    ) {
        crate::printing::print(document, complete);
    }

    fn announce(&mut self, message: &str, assertive: bool) -> bool {
        let key = self
            .windows
            .values()
            .find(|entry| entry.window.isKeyWindow());
        let Some(entry) = key.or_else(|| self.windows.get(&MAIN_WINDOW)) else {
            return false;
        };
        accessibility::announce_in(&entry.window, message, assertive);
        accessibility::screen_reader_active()
    }

    fn screen_reader_active(&self) -> bool {
        accessibility::screen_reader_active()
    }

    fn share(&mut self, window: WindowId, request: &pam_desktop_ui::shell::ShareRequest) -> bool {
        let Some(entry) = self
            .windows
            .get(&window)
            .or_else(|| self.windows.get(&MAIN_WINDOW))
        else {
            return false;
        };
        shell::share(self.mtm, &entry.host, request)
    }

    fn set_content_protection(&mut self, enabled: bool) -> bool {
        self.content_protected = enabled;
        let sharing = if enabled {
            NSWindowSharingType::None
        } else {
            NSWindowSharingType::ReadOnly
        };
        for entry in self.windows.values() {
            entry.window.setSharingType(sharing);
        }
        true
    }

    fn begin_text_drag(&mut self, window: WindowId, source: NodeId, text: &str) {
        let failed = BackendEvent::DragFinished {
            source,
            dropped: false,
        };
        let (Some(entry), Some(view)) = (self.windows.get(&window), self.views.get(&source)) else {
            return self.events.push(failed);
        };
        // A drag session starts from the press (or drag) being handled:
        // without a button down there is nothing to drag with.
        if NSEvent::pressedMouseButtons() & 1 == 0 {
            return self.events.push(failed);
        }
        let event = NSApplication::sharedApplication(self.mtm)
            .currentEvent()
            .filter(|event| {
                let kind = event.r#type();
                kind == NSEventType::LeftMouseDown || kind == NSEventType::LeftMouseDragged
            });
        let Some(event) = event.or_else(|| entry.window.press_event()) else {
            return self.events.push(failed);
        };
        let view = &view.node.view;
        let bounds = view.bounds();
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*ns_string(text)),
        );
        // What follows the pointer: a snapshot of the source view.
        let snapshot = view
            .bitmapImageRepForCachingDisplayInRect(bounds)
            .map(|bitmap| {
                view.cacheDisplayInRect_toBitmapImageRep(bounds, &bitmap);
                let image = NSImage::initWithSize(NSImage::alloc(), bounds.size);
                image.addRepresentation(&bitmap);
                image
            });
        let frame = entry.host.convertRect_fromView(bounds, Some(view));
        // SAFETY: the contents of a dragging item are an image, or nil.
        unsafe {
            item.setDraggingFrame_contents(
                frame,
                snapshot.as_deref().map(AsRef::<AnyObject>::as_ref),
            )
        };
        entry.host.set_drag_source(source);
        entry.host.beginDraggingSessionWithItems_event_source(
            &NSArray::from_retained_slice(&[item]),
            &event,
            ProtocolObject::from_ref(&*entry.host),
        );
    }

    fn set_media(&mut self, id: NodeId, state: &MediaState) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        let parent = entry.node.content_parent();
        if let Some(media) = &mut entry.media {
            media.apply(state, &parent);
        }
    }

    fn media_position(&self, id: NodeId) -> Option<(f64, f64)> {
        self.views.get(&id)?.media.as_ref()?.position()
    }

    fn set_web_view(&mut self, id: NodeId, state: &WebViewState) {
        let Some(entry) = self.views.get_mut(&id) else {
            return;
        };
        let parent = entry.node.content_parent();
        if let Some(web) = &mut entry.web {
            web.apply(state, &parent);
        }
    }

    fn set_drawing(&mut self, id: NodeId, commands: &[DrawCommand]) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.set_drawing(commands);
        }
    }

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        if let Some(scroll) = self
            .views
            .get_mut(&id)
            .and_then(|entry| entry.scroll.as_mut())
        {
            scroll.set_content(size);
        }
    }

    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig) {
        if let Some(scroll) = self
            .views
            .get_mut(&id)
            .and_then(|entry| entry.scroll.as_mut())
        {
            scroll.configure(config);
        }
    }

    fn scroll_to(&mut self, id: NodeId, offset: Point, animated: bool) {
        if let Some(scroll) = self.views.get(&id).and_then(|entry| entry.scroll.as_ref()) {
            scroll.scroll_to(offset, animated);
        }
    }

    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics {
        text::measure(block, max_width)
    }

    fn request_frame(&mut self) {
        // Any window's view will do: frames follow the display it is on.
        let host = self
            .windows
            .get(&MAIN_WINDOW)
            .or_else(|| self.windows.values().next())
            .map(|entry| &entry.host);
        self.frames.request(host.map(|host| -> &NSView { host }));
    }

    fn schedule_wake(&mut self, delay_ms: u32) {
        self.wake.schedule(&self.events, delay_ms);
    }

    fn clipboard_set_text(&mut self, text: &str) {
        self.shell.clipboard_set_text(text);
    }

    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>) {
        self.shell.clipboard_text(done);
    }

    fn show_alert(
        &mut self,
        window: WindowId,
        alert: &AlertConfig,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) {
        let parent = self
            .native_window(window)
            .filter(|window| window.isVisible());
        self.shell.show_alert(parent.as_deref(), alert, done);
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        self.forced_dark = dark;
        // Told before AppKit reacts: while an appearance is forced, the one
        // views see is not the system's.
        self.events.appearance_forced.set(dark.is_some());
        shell::set_appearance(self.mtm, dark);
    }

    fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        self.shell.set_menu_bar(menu);
    }

    fn popup_menu(
        &mut self,
        window: WindowId,
        position: Point,
        menu: &[MenuNode],
        done: Box<dyn FnOnce(Option<String>)>,
    ) {
        match self.windows.get(&window) {
            Some(entry) if !menu.is_empty() => {
                self.shell.popup_menu(&entry.host, position, menu, done)
            }
            _ => self.events.defer(move || done(None)),
        }
    }

    fn set_tray(&mut self, tray: Option<&TrayConfig>) {
        self.shell.set_tray(tray);
    }

    fn pick_files(
        &mut self,
        window: WindowId,
        config: &FileDialogConfig,
        done: Box<dyn FnOnce(Vec<std::path::PathBuf>)>,
    ) {
        let parent = self
            .native_window(window)
            .filter(|window| window.isVisible());
        self.shell.pick_files(parent.as_deref(), config, done);
    }

    fn notify(&mut self, notification: &NotificationConfig) {
        self.shell.notify(notification);
    }

    fn window_command(&mut self, window: WindowId, command: &WindowCommand) {
        if let Some(entry) = self.windows.get(&window) {
            shell::window_command(self.mtm, &entry.window, command);
        }
    }

    fn window_state(&self, window: WindowId) -> WindowState {
        self.windows
            .get(&window)
            .map(|entry| window::window_state(&entry.window))
            .unwrap_or_default()
    }

    fn set_shortcuts(&mut self, shortcuts: &[Shortcut]) {
        self.shell.set_shortcuts(shortcuts);
    }

    fn set_cursor(&mut self, id: NodeId, cursor: Cursor) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.set_cursor(cursor);
        }
    }

    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>) {
        if let Some(entry) = self.views.get(&id) {
            entry.node.view.setToolTip(text.map(ns_string).as_deref());
        }
    }

    fn begin_window_drag(&mut self, window: WindowId) {
        if let Some(entry) = self.windows.get(&window) {
            entry.window.begin_drag();
        }
    }

    fn quit(&mut self) {
        let windows: Vec<WindowId> = self.windows.keys().copied().collect();
        for window in windows {
            self.close_window(window);
        }
        self.release_services();
        let application = NSApplication::sharedApplication(self.mtm);
        application.stop(None);
        // `stop:` only takes effect when the run loop handles an event:
        // an empty one makes sure it does.
        let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            CGPoint::ZERO,
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        );
        if let Some(wake) = wake {
            application.postEvent_atStart(&wake, true);
        }
    }
}

impl Drop for AppKitBackend {
    fn drop(&mut self) {
        self.release_services();
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Events>]
    struct ApplicationDelegate;

    impl ApplicationDelegate {
        /// Outside the protocol block: the method joined the protocol in
        /// macOS 12, and debug builds refuse to register a protocol method
        /// the running system does not declare.
        #[unsafe(method(applicationSupportsSecureRestorableState:))]
        fn application_supports_secure_restorable_state(&self, _application: &NSApplication) -> bool {
            true
        }
    }

    unsafe impl NSObjectProtocol for ApplicationDelegate {}

    unsafe impl NSApplicationDelegate for ApplicationDelegate {
        /// Quit (menu, Dock, ⌘Q) asks the application like closing its main
        /// window does; the core quits through `Backend::quit`.
        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(&self, _sender: &NSApplication) -> NSApplicationTerminateReply {
            self.ivars().push(BackendEvent::WindowCloseRequested { window: MAIN_WINDOW });
            NSApplicationTerminateReply::TerminateCancel
        }

        /// The core decides when the application ends.
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn application_should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            false
        }

        /// A click on the Dock icon brings back a main window that was hidden.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn application_should_handle_reopen(&self, sender: &NSApplication, has_visible_windows: bool) -> bool {
            if !has_visible_windows {
                let main = sender.windows().iter().filter_map(|window| window.downcast::<PamWindow>().ok()).find(|window| window.id() == MAIN_WINDOW);
                if let Some(main) = main {
                    main.makeKeyAndOrderFront(None);
                }
            }
            true
        }

        /// A registered URL scheme was activated, at launch or later.
        #[unsafe(method(application:openURLs:))]
        fn application_open_urls(&self, _application: &NSApplication, urls: &NSArray<NSURL>) {
            for url in urls.iter() {
                if let Some(text) = url.absoluteString() {
                    self.ivars().push(BackendEvent::OpenUrl(text.to_string()));
                }
            }
        }
    }
);

/// Runs a PAM Desktop application on AppKit until the core quits.
///
/// The identity of a macOS application is its bundle's: `application_id`
/// is not used here.
pub fn run(application_id: &str, config: RuntimeConfig) -> ExitCode {
    let _ = application_id;
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("pam-desktop: the user interface must run on the main thread");
        return ExitCode::FAILURE;
    };
    let application = NSApplication::sharedApplication(mtm);
    // A program started outside a bundle still gets a Dock icon and a menu bar.
    application.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    let main = MainThread::new();
    let queue = EventQueue::new();
    let events = Events::new(Rc::clone(&queue), main.clone());
    Events::set_current(Some(&events));
    let inbox = Inbox::new(move || main.wake());

    let this = ApplicationDelegate::alloc(mtm).set_ivars(Rc::clone(&events));
    // SAFETY: `init` is the designated initializer of `NSObject`.
    let delegate: Retained<ApplicationDelegate> = unsafe { msg_send![super(this), init] };
    application.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    let backend = AppKitBackend::new(mtm, Rc::clone(&events));
    let shared = match Runtime::new(backend, inbox, queue, config) {
        Ok(runtime) => Shared::install(runtime),
        Err(error) => {
            eprintln!("pam-desktop: could not start the PHP worker: {error}");
            Events::set_current(None);
            return ExitCode::FAILURE;
        }
    };
    // While the runtime drains, events wait for the next turn of the run loop.
    let probe = Rc::downgrade(&shared);
    events.set_busy_probe(move || {
        probe
            .upgrade()
            .is_none_or(|shared| shared.with(|_| ()).is_none())
    });
    runtime::pump();
    crate::capture::schedule(mtm);

    application.run();

    runtime::set_pump(None);
    drop(shared);
    crate::events::discard_deferred();
    application.setDelegate(None);
    Events::set_current(None);
    ExitCode::SUCCESS
}
