//! `Backend` implementation on Win32 and the application entry point.
//!
//! Every method follows the same shape: mutate the retained scene under a
//! short state borrow, then (borrow released) do whatever touches windows.
//! Nothing here drains the runtime: events are queued and the window
//! procedures pump once the runtime is off the stack.

use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use pam_desktop_ui::accessibility::Accessibility;
use pam_desktop_ui::backend::{
    AlertConfig, Backend, BackendEvent, DrawCommand, ImageContent, ImageKey, ImageSource,
    InputConfig, MediaState, NodeId, ScrollConfig, TextContent, ViewClass, WebViewState,
    WindowConfig, WindowId, WindowMetrics,
};
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::runtime::{self, EventQueue, Inbox, Runtime, RuntimeConfig, Shared};
use pam_desktop_ui::shell::{
    Cursor, FileDialogConfig, MenuNode, NotificationConfig, Shortcut, TrayConfig, WindowCommand,
    WindowState,
};
use pam_desktop_ui::style::{Decoration, LayerState};
use pam_desktop_ui::text::{TextBlock, TextMetrics};
use windows::Win32::Foundation::HINSTANCE;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
use windows::Win32::UI::Controls::{
    ICC_STANDARD_CLASSES, ICC_WIN95_CLASSES, INITCOMMONCONTROLSEX, InitCommonControlsEx,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::{
    GetCurrentProcessExplicitAppUserModelID, SetCurrentProcessExplicitAppUserModelID,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, MSG, PostQuitMessage, TranslateMessage,
};
use windows::core::HSTRING;

use crate::app::App;
use crate::input::InputHost;
use crate::scene::TextState;
use crate::text::text_origin;

pub struct Win32Backend {
    app: Rc<App>,
}

impl Win32Backend {
    #[must_use]
    pub fn new(app: Rc<App>) -> Self {
        Self { app }
    }
}

impl Backend for Win32Backend {
    // ---- windows -------------------------------------------------------

    fn open_window(&mut self, window: WindowId, config: &WindowConfig) {
        crate::window::open(&self.app, window, config);
    }

    fn close_window(&mut self, window: WindowId) {
        crate::window::close(&self.app, window);
    }

    fn window_metrics(&self, window: WindowId) -> WindowMetrics {
        crate::window::metrics(&self.app, window)
    }

    fn set_window_root(&mut self, window: WindowId, root: Option<NodeId>) {
        if let Some(mut state) = self.app.state() {
            state.scene.set_layers(window, Some(root), None);
        }
        crate::window::present(&self.app, window);
    }

    fn set_window_overlays(&mut self, window: WindowId, overlays: &[NodeId]) {
        if let Some(mut state) = self.app.state() {
            state.scene.set_layers(window, None, Some(overlays));
        }
    }

    // ---- views ---------------------------------------------------------

    fn create_view(&mut self, id: NodeId, class: ViewClass) {
        // An id created again replaces its view: the old one releases its
        // native resources like any destroyed view.
        if self
            .app
            .state()
            .is_some_and(|state| state.scene.views.contains_key(&id))
        {
            self.destroy_view(id);
        }
        let Some(mut state) = self.app.state() else {
            return;
        };
        state.scene.create(id, class);
        let scene = &mut state.scene;
        let Some(view) = scene.views.get_mut(&id) else {
            return;
        };
        match class {
            ViewClass::Input | ViewClass::TextArea => {
                view.input = Some(Box::new(InputHost::new(class == ViewClass::TextArea)));
                scene.natives.push(id);
            }
            ViewClass::Media => {
                view.media = Some(Box::new(crate::media::MediaPlayer::new()));
                scene.players.push(id);
            }
            ViewClass::WebView => {
                view.web = Some(Box::new(crate::webview::WebHost::new(id)));
                scene.natives.push(id);
            }
            ViewClass::Box | ViewClass::Scroll | ViewClass::Surface => {}
        }
    }

    fn destroy_view(&mut self, id: NodeId) {
        let removed = self
            .app
            .state()
            .and_then(|mut state| state.scene.destroy(id));
        // Native resources are released with the state free again.
        if let Some(mut view) = removed {
            if let Some(input) = view.input.take() {
                crate::input::release(&self.app, input);
            }
            if let Some(media) = view.media.take() {
                crate::media::release(&self.app, media);
            }
            if let Some(web) = view.web.take() {
                crate::webview::release(&self.app, web);
            }
            crate::accessibility::view_destroyed(&self.app, id);
        }
    }

    fn set_children(&mut self, parent: NodeId, children: &[NodeId]) {
        if let Some(mut state) = self.app.state() {
            state.scene.set_children(parent, children);
        }
    }

    fn set_frame(&mut self, id: NodeId, frame: Rect) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(view) = state.scene.views.get_mut(&id) else {
            return;
        };
        if view.frame == frame {
            return;
        }
        let resized = view.frame.size() != frame.size();
        view.frame = frame;
        let scrolls = view.scroll.is_some();
        state.scene.damage_subtree(id);
        if resized && scrolls {
            // A larger viewport may leave the offset beyond the content,
            // and a smaller one may make a deferred offset reachable.
            state.scrolling().settle(id);
        }
    }

    fn set_decoration(&mut self, id: NodeId, decoration: &Decoration) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(view) = state.scene.views.get(&id) else {
            return;
        };
        // Clipping and filters change what the whole subtree shows.
        let reach_changed = view.decoration.clip != decoration.clip
            || view.decoration.blur != decoration.blur
            || view.decoration.color_matrix != decoration.color_matrix;
        // The old shadows must be repainted away too.
        state.scene.damage_view(id);
        let Some(view) = state.scene.views.get_mut(&id) else {
            return;
        };
        let recolored = view.decoration.background != decoration.background;
        view.decoration.clone_from(decoration);
        if let Some(cache) = &mut view.cache {
            cache.invalidate();
        }
        if recolored {
            // Text controls paint their own background to match the box behind them.
            state.scene.touch_natives(id);
        }
        if reach_changed {
            state.scene.damage_subtree(id);
            if let Some(window) = state.scene.window_of(id) {
                state.scene.damage_all(window);
            }
        } else {
            state.scene.damage_view(id);
        }
    }

    fn set_layer(&mut self, id: NodeId, layer: &LayerState) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(view) = state.scene.views.get_mut(&id) else {
            return;
        };
        if view.layer == *layer {
            return;
        }
        view.layer = *layer;
        state.scene.damage_subtree(id);
    }

    fn set_text(&mut self, id: NodeId, text: Option<&TextContent>) {
        {
            let Some(mut state) = self.app.state() else {
                return;
            };
            let Some(view) = state.scene.views.get_mut(&id) else {
                return;
            };
            view.text = text.map(|content| TextState {
                content: content.clone(),
                layout: None,
                wrapped_to: f32::NAN,
                colored_for: 0,
                selection: None,
            });
            state.scene.damage_content(id);
        }
        // The text is the accessible name unless a label was given.
        crate::accessibility::content_changed(&self.app, id);
    }

    fn set_focusable(&mut self, id: NodeId, focusable: bool) {
        if let Some(mut state) = self.app.state()
            && let Some(view) = state.scene.views.get_mut(&id)
        {
            view.focusable = focusable;
        }
    }

    fn set_accessibility(&mut self, id: NodeId, accessibility: Option<&Accessibility>) {
        let previous = {
            let Some(mut state) = self.app.state() else {
                return;
            };
            let Some(view) = state.scene.views.get_mut(&id) else {
                return;
            };
            std::mem::replace(
                &mut view.accessibility,
                accessibility.cloned().map(Box::new),
            )
        };
        crate::accessibility::content_changed(&self.app, id);
        crate::accessibility::state_changed(&self.app, id, previous.as_deref());
    }

    fn focus(&mut self, id: Option<NodeId>) {
        crate::focus::set(&self.app, id);
    }

    // ---- text input ----------------------------------------------------

    fn set_input_config(&mut self, id: NodeId, config: &InputConfig) {
        crate::input::configure(&self.app, id, config);
    }

    fn set_input_text(&mut self, id: NodeId, text: &str, selection: Option<(u32, u32)>) {
        crate::input::set_text(&self.app, id, text, selection);
    }

    fn set_input_selection(&mut self, id: NodeId, start: u32, end: u32) {
        crate::input::set_selection(&self.app, id, start, end);
    }

    // ---- images --------------------------------------------------------

    fn load_image(&mut self, key: ImageKey, source: ImageSource, max_dimension: Option<u32>) {
        let mailbox = Arc::clone(&self.app.mailbox);
        let started = self
            .app
            .state()
            .is_some_and(|mut state| state.images.load(&mailbox, key, source, max_dimension));
        if !started {
            self.app.emit(BackendEvent::ImageFailed {
                key,
                message: "image decoder could not start".into(),
            });
        }
    }

    fn release_image(&mut self, key: ImageKey) {
        if let Some(mut state) = self.app.state() {
            state.images.release(key);
        }
    }

    fn set_image(&mut self, id: NodeId, image: Option<&ImageContent>) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(view) = state.scene.views.get_mut(&id) else {
            return;
        };
        view.image = image.copied();
        state.scene.damage_view(id);
    }

    fn text_index_at(&self, id: NodeId, point: Point) -> Option<u32> {
        let mut state = self.app.state()?;
        let state = &mut *state;
        let view = state.scene.views.get_mut(&id)?;
        let size = view.frame.size();
        let text = view.text.as_mut()?;
        let insets = text.content.insets;
        let available = (size.width - insets.horizontal()).max(0.0);
        // The layout that is (or will be) painted answers, so the index
        // matches what the user sees.
        if text.layout.is_none() || text.wrapped_to != available {
            text.layout = state
                .text
                .build(&state.graphics.dwrite, &text.content.block, available);
            text.wrapped_to = available;
            text.colored_for = 0;
        }
        let layout = text.layout.as_ref()?;
        let origin = text_origin(size, insets, text.content.vertical, layout.height);
        Some(layout.index_at(Point::new(point.x - origin.x, point.y - origin.y)))
    }

    fn set_text_selection(&mut self, id: NodeId, range: Option<(u32, u32)>) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(text) = state
            .scene
            .views
            .get_mut(&id)
            .and_then(|view| view.text.as_mut())
        else {
            return;
        };
        let range = range.filter(|(start, end)| start < end);
        if text.selection == range {
            return;
        }
        text.selection = range;
        // Selected glyphs take the highlight text color.
        text.colored_for = 0;
        state.scene.damage_content(id);
    }

    fn begin_text_drag(&mut self, window: WindowId, source: NodeId, text: &str) {
        crate::dragdrop::begin_text_drag(&self.app, window, source, text);
    }

    fn announce(&mut self, message: &str, assertive: bool) -> bool {
        crate::accessibility::announce(&self.app, message, assertive)
    }

    fn set_content_protection(&mut self, enabled: bool) -> bool {
        crate::window::set_content_protection(&self.app, enabled)
    }

    fn screen_reader_active(&self) -> bool {
        crate::accessibility::screen_reader_active()
    }

    fn set_media(&mut self, id: NodeId, state: &MediaState) {
        crate::media::apply(&self.app, id, state);
    }

    fn media_position(&self, id: NodeId) -> Option<(f64, f64)> {
        crate::media::position(&self.app, id)
    }

    fn set_web_view(&mut self, id: NodeId, state: &WebViewState) {
        crate::webview::apply(&self.app, id, state);
    }

    fn set_drawing(&mut self, id: NodeId, commands: &[DrawCommand]) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(view) = state.scene.views.get_mut(&id) else {
            return;
        };
        view.drawing.clear();
        view.drawing.extend_from_slice(commands);
        if let Some(cache) = &mut view.cache {
            cache.invalidate_drawing();
        }
        state.scene.damage_view(id);
    }

    // ---- scrolling -----------------------------------------------------

    fn set_scroll_content(&mut self, id: NodeId, size: Size) {
        if let Some(mut state) = self.app.state() {
            state.scrolling().set_content(id, size);
            state.schedule_scroll_work();
        }
    }

    fn set_scroll_config(&mut self, id: NodeId, config: &ScrollConfig) {
        let Some(mut state) = self.app.state() else {
            return;
        };
        let Some(scroll) = state
            .scene
            .views
            .get_mut(&id)
            .and_then(|view| view.scroll.as_mut())
        else {
            return;
        };
        if scroll.config == *config {
            return;
        }
        scroll.config = *config;
        if !config.enabled || !config.shows_indicators {
            scroll.bar_opacity = 0.0;
        }
        state.scene.damage_view(id);
    }

    fn scroll_to(&mut self, id: NodeId, offset: Point, animated: bool) {
        if let Some(mut state) = self.app.state() {
            state.scrolling().scroll_to(id, offset, animated);
            state.schedule_scroll_work();
        }
    }

    // ---- text ----------------------------------------------------------

    fn measure_text(&mut self, block: &TextBlock, max_width: f32) -> TextMetrics {
        let Some(mut state) = self.app.state() else {
            return TextMetrics::default();
        };
        let state = &mut *state;
        state.text.measure(&state.graphics.dwrite, block, max_width)
    }

    // ---- frames --------------------------------------------------------

    fn request_frame(&mut self) {
        if let Some(mut state) = self.app.state() {
            state.core_frame_requested = true;
            state.request_tick();
        }
    }

    fn schedule_wake(&mut self, delay_ms: u32) {
        self.app.schedule_wake(delay_ms);
    }

    // ---- system services -----------------------------------------------

    fn clipboard_set_text(&mut self, text: &str) {
        crate::shell::clipboard_set_text(&self.app, text);
    }

    fn clipboard_text(&mut self, done: Box<dyn FnOnce(Option<String>)>) {
        crate::shell::clipboard_text(&self.app, done);
    }

    fn show_alert(
        &mut self,
        window: WindowId,
        alert: &AlertConfig,
        done: Box<dyn FnOnce(Option<usize>)>,
    ) {
        crate::shell::show_alert(&self.app, window, alert, done);
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        crate::shell::set_appearance(&self.app, dark);
    }

    // ---- desktop shell -------------------------------------------------

    fn set_menu_bar(&mut self, menu: &[MenuNode]) {
        crate::shell::set_menu_bar(&self.app, menu);
    }

    fn popup_menu(
        &mut self,
        window: WindowId,
        position: Point,
        menu: &[MenuNode],
        done: Box<dyn FnOnce(Option<String>)>,
    ) {
        crate::shell::popup_menu(&self.app, window, position, menu, done);
    }

    fn set_tray(&mut self, tray: Option<&TrayConfig>) {
        crate::shell::set_tray(&self.app, tray);
    }

    fn pick_files(
        &mut self,
        window: WindowId,
        config: &FileDialogConfig,
        done: Box<dyn FnOnce(Vec<PathBuf>)>,
    ) {
        crate::shell::pick_files(&self.app, window, config, done);
    }

    fn notify(&mut self, notification: &NotificationConfig) {
        crate::shell::notify(&self.app, notification);
    }

    fn window_command(&mut self, window: WindowId, command: &WindowCommand) {
        crate::window::command(&self.app, window, command);
    }

    fn window_state(&self, window: WindowId) -> WindowState {
        crate::window::state_of(&self.app, window)
    }

    fn set_shortcuts(&mut self, shortcuts: &[Shortcut]) {
        crate::shell::set_shortcuts(&self.app, shortcuts);
    }

    fn set_cursor(&mut self, id: NodeId, cursor: Cursor) {
        if let Some(mut state) = self.app.state()
            && let Some(view) = state.scene.views.get_mut(&id)
        {
            view.cursor = cursor;
        }
    }

    fn set_tooltip(&mut self, id: NodeId, text: Option<&str>) {
        if let Some(mut state) = self.app.state() {
            if let Some(view) = state.scene.views.get_mut(&id) {
                view.tooltip = text.map(str::to_owned);
            }
            // The armed tooltip is re-read on the next pointer movement.
            for entry in state.scene.windows.values_mut() {
                if entry.tooltip_node == Some(id) {
                    entry.tooltip_node = None;
                }
            }
        }
    }

    fn begin_window_drag(&mut self, window: WindowId) {
        crate::window::begin_drag(&self.app, window);
    }

    // ---- application ---------------------------------------------------

    fn quit(&mut self) {
        self.app.quitting.set(true);
        crate::window::close_all(&self.app);
        // SAFETY: ends the message loop of this (the UI) thread.
        unsafe { PostQuitMessage(0) };
    }
}

/// Process-wide setup that must precede the first window.
fn prepare_process(application_id: &str) {
    // SAFETY: process and thread initialization calls without preconditions
    // beyond running on the thread that will own the windows.
    unsafe {
        // Fails when the executable's manifest already declared it: fine.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // OLE (a single-threaded apartment) for drag and drop, the
        // clipboard, the shell dialogs and UI Automation.
        let _ = OleInitialize(None);
        let controls = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        let _ = InitCommonControlsEx(&controls);
        // The taskbar and notifications identify the application by this
        // id, unless the launcher (a shortcut, a package) already set one.
        match GetCurrentProcessExplicitAppUserModelID() {
            Ok(existing) => CoTaskMemFree(Some(existing.0.cast())),
            Err(_) => {
                let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(application_id));
            }
        }
    }
}

/// Runs a PAM Desktop application on Win32 until its window closes.
pub fn run(application_id: &str, config: RuntimeConfig) -> ExitCode {
    // One instance per application: a second launch hands its link over.
    let instance_lock = if config.single_instance {
        match crate::instance::acquire(application_id, config.launch_url.as_deref()) {
            crate::instance::Launch::Handed => return ExitCode::SUCCESS,
            crate::instance::Launch::First(lock) => Some(lock),
        }
    } else {
        None
    };
    let single_instance = config.single_instance;
    prepare_process(application_id);
    // SAFETY: a null module name is the executable itself.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map(Into::into)
        .unwrap_or_default();
    let events = EventQueue::new();
    let app = match App::install(instance, application_id, Rc::clone(&events)) {
        Ok(app) => app,
        Err(error) => {
            eprintln!(
                "pam-desktop: could not start the Windows backend: {}",
                error.message()
            );
            // SAFETY: balances the OleInitialize of `prepare_process`.
            unsafe { OleUninitialize() };
            return ExitCode::FAILURE;
        }
    };
    crate::window::register_class(&app);
    let instance_window = if single_instance {
        crate::instance::listen(&app)
    } else {
        None
    };
    let mailbox = Arc::clone(&app.mailbox);
    // Other threads reach the UI thread by posting to the message window,
    // whose handler drains the runtime.
    let inbox = Inbox::new(move || mailbox.wake());
    let mut failed = false;
    let shared = match Runtime::new(Win32Backend::new(Rc::clone(&app)), inbox, events, config) {
        Ok(runtime) => {
            let shared = Shared::install(runtime);
            // The first drain mounts the application.
            app.mark_pending();
            app.pump();
            Some(shared)
        }
        Err(error) => {
            eprintln!("pam-desktop: could not start the PHP worker: {error}");
            failed = true;
            crate::window::close_all(&app);
            None
        }
    };
    // Without its main window the application could never be closed.
    if !failed && app.with_state(|state| state.scene.windows.is_empty()) != Some(false) {
        eprintln!("pam-desktop: could not create the application window");
        failed = true;
    }

    let mut exit_code = 0;
    if !failed {
        let mut message = MSG::default();
        // SAFETY: the standard message loop of the UI thread; `message` is a
        // live local for every call.
        unsafe {
            loop {
                let result = GetMessageW(&mut message, None, 0, 0).0;
                if result == 0 {
                    exit_code = message.wParam.0 as i32;
                    break;
                }
                if result < 0 {
                    failed = true;
                    break;
                }
                if !crate::shell::translate_accelerator(&app, &message) {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }

    runtime::set_pump(None);
    // The runtime (and the worker) go first: its backend handle is the
    // last user of the scene.
    drop(shared);
    crate::instance::stop(instance_window);
    crate::shell::shutdown(&app);
    crate::media::shutdown(&app);
    crate::webview::shutdown();
    app.uninstall();
    drop(app);
    // SAFETY: balances the OleInitialize of `prepare_process`, after every
    // COM object of this thread was released.
    unsafe { OleUninitialize() };
    drop(instance_lock);
    if failed || exit_code != 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
