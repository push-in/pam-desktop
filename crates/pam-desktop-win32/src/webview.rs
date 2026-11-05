//! The page of a `WebView` view, through WebView2 (the Evergreen runtime
//! of Edge) in a child window over the node.
//!
//! WebView2 is created asynchronously: the environment once per process,
//! then one controller per view, both completing from the message loop.
//! The state the core asked for is kept and applied when the page exists.
//! A missing runtime is reported as `WebViewEvent::Failed`.

use std::rc::Rc;

use pam_desktop_ui::backend::{NodeId, WebViewState, WindowId};
use windows::Win32::Foundation::HWND;

use crate::app::App;
use crate::scene::Placement;

#[cfg(feature = "webview")]
pub use enabled::WebHost;
#[cfg(feature = "webview")]
pub use enabled::{
    apply, close_retired, detach, focus, parent_destroyed, place, release, shutdown, window_moved,
};

#[cfg(not(feature = "webview"))]
pub use disabled::WebHost;
#[cfg(not(feature = "webview"))]
pub use disabled::{
    apply, close_retired, detach, focus, parent_destroyed, place, release, shutdown, window_moved,
};

#[cfg(not(feature = "webview"))]
mod disabled {
    use pam_desktop_ui::backend::{BackendEvent, WebSource, WebViewEvent};

    use super::*;

    /// Built without a web engine: every page fails to load.
    pub struct WebHost {
        /// The source the failure was last reported for.
        reported: WebSource,
    }

    impl WebHost {
        pub fn new(_id: NodeId) -> Self {
            Self {
                reported: WebSource::None,
            }
        }
    }

    pub fn place(_app: &Rc<App>, _id: NodeId, _placement: &Placement) {}

    pub fn detach(_app: &Rc<App>, _id: NodeId, _parent: HWND) {}

    pub fn parent_destroyed(_host: &mut WebHost, _parent: HWND) {}

    pub fn close_retired() {}

    pub fn window_moved(_app: &Rc<App>, _window: WindowId) {}

    pub fn focus(_app: &Rc<App>, _id: NodeId) -> bool {
        false
    }

    pub fn shutdown() {}

    pub fn apply(app: &Rc<App>, id: NodeId, state: &WebViewState) {
        // Each new page fails once, like a page that could not be loaded.
        let changed = app
            .with_state(|state_| {
                let host = state_.scene.views.get_mut(&id)?.web.as_mut()?;
                (host.reported != state.source).then(|| host.reported = state.source.clone())
            })
            .flatten()
            .is_some();
        if changed && state.source != WebSource::None {
            app.emit(BackendEvent::WebView {
                id,
                event: WebViewEvent::Failed("WebView2 is not available in this build".into()),
            });
        }
    }

    // The scene owns it boxed and hands the box back.
    #[allow(clippy::boxed_local)]
    pub fn release(_app: &Rc<App>, _host: Box<WebHost>) {}
}

#[cfg(feature = "webview")]
mod enabled {
    use std::cell::RefCell;

    use pam_desktop_ui::backend::{BackendEvent, WebSource, WebViewEvent};
    use pam_desktop_ui::webview::{BRIDGE_SCRIPT, navigation_allowed};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT, COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS,
        COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC, COREWEBVIEW2_PERMISSION_STATE_DENY,
        COREWEBVIEW2_WEB_ERROR_STATUS, COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED,
        COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN, CreateCoreWebView2EnvironmentWithOptions,
        ICoreWebView2, ICoreWebView2Controller, ICoreWebView2Environment,
        ICoreWebView2EnvironmentOptions, ICoreWebView2NavigationCompletedEventArgs2,
        ICoreWebView2Settings2,
    };
    use webview2_com::{
        AddScriptToExecuteOnDocumentCreatedCompletedHandler, CoreWebView2EnvironmentOptions,
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, ExecuteScriptCompletedHandler,
        FocusChangedEventHandler, MoveFocusRequestedEventHandler, NavigationCompletedEventHandler,
        NavigationStartingEventHandler, NewWindowRequestedEventHandler,
        PermissionRequestedEventHandler, ProcessFailedEventHandler, WebMessageReceivedEventHandler,
    };
    use windows::Win32::Foundation::{LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, SW_HIDE, SW_SHOWNA,
        SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WNDCLASSEXW,
        WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    };
    use windows::core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR, w};

    use super::*;
    use crate::util::pixel_rect_nearest;

    const HOST_CLASS: PCWSTR = w!("PamDesktopWebHost");
    /// Keeps a page from scrolling (`scroll_enabled == false`), or lets it again.
    const SCROLL_LOCK: &str = "(function(lock){var id='__pam_scroll_lock';var style=document.getElementById(id);if(lock){if(!style&&document.documentElement){style=document.createElement('style');style.id=id;style.textContent='html,body{overflow:hidden !important;}';document.documentElement.appendChild(style);}}else if(style){style.remove();}})";

    /// The WebView2 environment shared by every page of the process.
    #[derive(Default)]
    struct Shared {
        environment: Option<ICoreWebView2Environment>,
        creating: bool,
        /// Why the environment could not be created.
        failure: Option<String>,
        /// Views waiting for the environment.
        waiting: Vec<NodeId>,
        class_registered: bool,
        /// Controllers of destroyed windows, closed once the state is free.
        retired: Vec<ICoreWebView2Controller>,
    }

    thread_local! {
        static SHARED: RefCell<Shared> = RefCell::new(Shared::default());
    }

    pub struct WebHost {
        id: NodeId,
        /// The child window the page lives in.
        container: Option<HWND>,
        /// The top-level window the container was created in.
        parent: HWND,
        controller: Option<ICoreWebView2Controller>,
        webview: Option<ICoreWebView2>,
        /// A controller is being created for `container`.
        creating: bool,
        /// Why the page could not be created.
        failure: Option<String>,
        /// What the page should be; `applied` is what it was last brought to.
        wanted: Option<WebViewState>,
        applied: Option<WebViewState>,
        /// A main-frame page finished and nothing new started since.
        loaded: bool,
        /// The engine's own user agent, to restore it.
        default_user_agent: String,
        placement: Option<Placement>,
        /// A failure was already reported for the current request.
        failed: bool,
    }

    impl WebHost {
        pub fn new(id: NodeId) -> Self {
            Self {
                id,
                container: None,
                parent: HWND::default(),
                controller: None,
                webview: None,
                creating: false,
                failure: None,
                wanted: None,
                applied: None,
                loaded: false,
                default_user_agent: String::new(),
                placement: None,
                failed: false,
            }
        }

        /// Forgets the page (its window is gone or going).
        fn reset(&mut self) -> Option<ICoreWebView2Controller> {
            self.container = None;
            self.webview = None;
            self.creating = false;
            self.applied = None;
            self.loaded = false;
            self.placement = None;
            self.controller.take()
        }
    }

    fn host_mut(state: &mut crate::app::State, id: NodeId) -> Option<&mut WebHost> {
        state.scene.views.get_mut(&id)?.web.as_deref_mut()
    }

    /// Copies a string WebView2 allocated and frees it.
    fn take_string(text: PWSTR) -> String {
        if text.is_null() {
            return String::new();
        }
        // SAFETY: WebView2 returns NUL-terminated strings allocated with the
        // COM allocator, which the caller frees.
        unsafe {
            let copy = text.to_string().unwrap_or_default();
            CoTaskMemFree(Some(text.0.cast()));
            copy
        }
    }

    fn source_of(webview: &ICoreWebView2) -> String {
        let mut uri = PWSTR::null();
        // SAFETY: the out pointer refers to a live local.
        if unsafe { webview.Source(&mut uri) }.is_ok() {
            take_string(uri)
        } else {
            String::new()
        }
    }

    fn execute(webview: &ICoreWebView2, script: &str) {
        if script.is_empty() {
            return;
        }
        let handler = ExecuteScriptCompletedHandler::create(Box::new(|_, _| Ok(())));
        // SAFETY: COM call on the live page; the script string outlives the call.
        unsafe {
            let _ = webview.ExecuteScript(&HSTRING::from(script), &handler);
        }
    }

    fn lock_scroll(webview: &ICoreWebView2, scroll_enabled: bool) {
        execute(
            webview,
            &format!(
                "{SCROLL_LOCK}({});",
                if scroll_enabled { "false" } else { "true" }
            ),
        );
    }

    fn close(controller: &ICoreWebView2Controller) {
        // SAFETY: COM call on a live controller; no state borrow is held
        // (closing destroys windows of the page).
        unsafe {
            let _ = controller.Close();
        }
    }

    /// Reports an event of the page. WebView2 is calling us: the runtime is
    /// drained after it returns, so the core can never close the page
    /// from inside its own callback.
    fn report(app: &Rc<App>, id: NodeId, event: WebViewEvent) {
        app.emit_later(BackendEvent::WebView { id, event });
    }

    /// Reports a failure once per requested page.
    fn fail(app: &Rc<App>, id: NodeId, message: String) {
        let first = app.with_state(|state| {
            let host = host_mut(state, id)?;
            (!std::mem::replace(&mut host.failed, true)).then_some(())
        });
        if first == Some(Some(())) {
            report(app, id, WebViewEvent::Failed(message));
        }
    }

    fn user_data_folder(app: &App) -> String {
        // Next to the executable is the default, which is rarely writable.
        let base = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(&app.application_id)
            .join("WebView2")
            .display()
            .to_string()
    }

    /// Starts creating the shared environment; views in `waiting` continue
    /// when it completes. `autoplay` lets media play without a user gesture
    /// (a process-wide browser switch: the first page decides).
    fn create_environment(app: &Rc<App>, autoplay: bool) {
        let weak = Rc::downgrade(app);
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let Some(app) = weak.upgrade() else {
                    return Ok(());
                };
                let waiting = SHARED.with(|shared| {
                    let mut shared = shared.borrow_mut();
                    shared.creating = false;
                    match (&result, environment) {
                        (Ok(()), Some(environment)) => shared.environment = Some(environment),
                        (Err(error), _) => {
                            shared.failure =
                                Some(format!("WebView2 could not start ({})", error.message()))
                        }
                        (Ok(()), None) => {
                            shared.failure = Some(String::from("WebView2 could not start"))
                        }
                    }
                    std::mem::take(&mut shared.waiting)
                });
                for id in waiting {
                    create_controller(&app, id);
                }
                Ok(())
            },
        ));
        let folder = HSTRING::from(user_data_folder(app));
        let options = CoreWebView2EnvironmentOptions::default();
        if autoplay {
            // SAFETY: the options object is not shared yet.
            unsafe {
                options.set_additional_browser_arguments(String::from(
                    "--autoplay-policy=no-user-gesture-required",
                ))
            };
        }
        let options: ICoreWebView2EnvironmentOptions = options.into();
        // SAFETY: the folder string, the options and the handler outlive the
        // call; a null browser folder selects the installed Evergreen runtime.
        let started = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &folder, &options, &handler)
        };
        if let Err(error) = started {
            // The loader fails at once when no runtime is installed.
            let waiting = SHARED.with(|shared| {
                let mut shared = shared.borrow_mut();
                shared.creating = false;
                shared.failure = Some(format!(
                    "The WebView2 runtime is not installed ({})",
                    error.message()
                ));
                std::mem::take(&mut shared.waiting)
            });
            for id in waiting {
                create_controller(app, id);
            }
        }
    }

    /// The page could not be created: remembered, so later requests fail too.
    fn creation_failed(app: &Rc<App>, id: NodeId, container: Option<HWND>, message: String) {
        let current = app.with_state(|state| {
            let host = host_mut(state, id)?;
            // A callback for a container that was replaced meanwhile is stale.
            if container.is_some() && host.container != container {
                return None;
            }
            host.creating = false;
            host.failure = Some(message.clone());
            Some(())
        });
        if current == Some(Some(())) {
            fail(app, id, message);
        }
    }

    /// Creates the page of `id` in its container, once the environment exists.
    fn create_controller(app: &Rc<App>, id: NodeId) {
        enum Step {
            Wait,
            Start,
            Fail(String),
            Create(ICoreWebView2Environment),
        }
        let step = SHARED.with(|shared| {
            let mut shared = shared.borrow_mut();
            if let Some(environment) = &shared.environment {
                return Step::Create(environment.clone());
            }
            if let Some(failure) = &shared.failure {
                return Step::Fail(failure.clone());
            }
            if !shared.waiting.contains(&id) {
                shared.waiting.push(id);
            }
            if std::mem::replace(&mut shared.creating, true) {
                Step::Wait
            } else {
                Step::Start
            }
        });
        let environment = match step {
            Step::Wait => return,
            Step::Start => {
                let autoplay = app
                    .with_state(|state| {
                        host_mut(state, id)
                            .and_then(|host| host.wanted.as_ref().map(|wanted| wanted.inline_media))
                    })
                    .flatten()
                    .unwrap_or(true);
                return create_environment(app, autoplay);
            }
            Step::Fail(message) => return creation_failed(app, id, None, message),
            Step::Create(environment) => environment,
        };
        let Some(Some(container)) =
            app.with_state(|state| host_mut(state, id).and_then(|host| host.container))
        else {
            return;
        };
        let weak = Rc::downgrade(app);
        let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                let Some(app) = weak.upgrade() else {
                    return Ok(());
                };
                match (result, controller) {
                    (Ok(()), Some(controller)) => controller_ready(&app, id, container, controller),
                    (Err(error), _) => creation_failed(
                        &app,
                        id,
                        Some(container),
                        format!("WebView2 could not create the page ({})", error.message()),
                    ),
                    (Ok(()), None) => creation_failed(
                        &app,
                        id,
                        Some(container),
                        String::from("WebView2 could not create the page"),
                    ),
                }
                Ok(())
            },
        ));
        // SAFETY: the container is a live child window of this thread.
        if let Err(error) = unsafe { environment.CreateCoreWebView2Controller(container, &handler) }
        {
            creation_failed(
                app,
                id,
                Some(container),
                format!("WebView2 could not create the page ({})", error.message()),
            );
        }
    }

    /// The page exists: wire its events, then bring it to the wanted state.
    fn controller_ready(
        app: &Rc<App>,
        id: NodeId,
        container: HWND,
        controller: ICoreWebView2Controller,
    ) {
        // SAFETY: COM call on the freshly created controller.
        let Ok(webview) = (unsafe { controller.CoreWebView2() }) else {
            close(&controller);
            return creation_failed(
                app,
                id,
                Some(container),
                String::from("WebView2 could not create the page"),
            );
        };
        let default_user_agent = {
            let mut agent = PWSTR::null();
            // SAFETY: COM calls on the live page; the out pointer refers to a live local.
            unsafe {
                match webview
                    .Settings()
                    .and_then(|settings| settings.cast::<ICoreWebView2Settings2>())
                {
                    Ok(settings) if settings.UserAgent(&mut agent).is_ok() => take_string(agent),
                    _ => String::new(),
                }
            }
        };
        let adopted = app.with_state(|state| {
            let host = host_mut(state, id)?;
            // The view moved to another window (or lost its window) while
            // the page was being created: this page is for a dead container.
            if host.container != Some(container) || host.controller.is_some() {
                return None;
            }
            host.creating = false;
            host.failure = None;
            host.controller = Some(controller.clone());
            host.webview = Some(webview.clone());
            host.default_user_agent = default_user_agent;
            host.applied = None;
            // The container is placed again, now with a page to size and show.
            host.placement = None;
            if let Some(window) = state.scene.window_of(id) {
                state.scene.damage_all(window);
            }
            Some(())
        });
        if adopted != Some(Some(())) {
            return close(&controller);
        }

        let mut token = 0_i64;
        // SAFETY: COM calls on the live page; handlers and strings outlive
        // the calls that register them, and out pointers refer to live locals.
        unsafe {
            if let Ok(settings) = webview.Settings() {
                let _ = settings.SetIsWebMessageEnabled(true);
                let _ = settings.SetAreDevToolsEnabled(false);
                let _ = settings.SetIsStatusBarEnabled(false);
            }

            let weak = Rc::downgrade(app);
            let starting = NavigationStartingEventHandler::create(Box::new(move |_, arguments| {
                let (Some(app), Some(arguments)) = (weak.upgrade(), arguments) else {
                    return Ok(());
                };
                let mut uri = PWSTR::null();
                arguments.Uri(&mut uri)?;
                let url = take_string(uri);
                let allowed = app
                    .with_state(|state| {
                        let host = host_mut(state, id)?;
                        let allowed = host
                            .wanted
                            .as_ref()
                            .is_none_or(|wanted| navigation_allowed(&wanted.allowed_hosts, &url));
                        if allowed {
                            host.loaded = false;
                        }
                        Some(allowed)
                    })
                    .flatten()
                    .unwrap_or(true);
                if !allowed {
                    arguments.SetCancel(true)?;
                    report(&app, id, WebViewEvent::Blocked { url });
                }
                Ok(())
            }));
            let _ = webview.add_NavigationStarting(&starting, &mut token);

            let weak = Rc::downgrade(app);
            let completed =
                NavigationCompletedEventHandler::create(Box::new(move |webview, arguments| {
                    let (Some(app), Some(webview), Some(arguments)) =
                        (weak.upgrade(), webview, arguments)
                    else {
                        return Ok(());
                    };
                    let mut success = BOOL(0);
                    arguments.IsSuccess(&mut success)?;
                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                    let _ = arguments.WebErrorStatus(&mut status);
                    // An HTTP error status still shows a page: that is a load.
                    let mut http_status = 0_i32;
                    if let Ok(detailed) =
                        arguments.cast::<ICoreWebView2NavigationCompletedEventArgs2>()
                    {
                        let _ = detailed.HttpStatusCode(&mut http_status);
                    }
                    let shown = success.as_bool()
                        || (status == COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN && http_status >= 400);
                    if !shown {
                        // A stopped or replaced load is not a failure of the page.
                        if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                            report(&app, id, WebViewEvent::Failed(error_text(status)));
                        }
                        return Ok(());
                    }
                    let wanted = app
                        .with_state(|state| {
                            let host = host_mut(state, id)?;
                            host.loaded = true;
                            host.wanted.clone()
                        })
                        .flatten();
                    if let Some(wanted) = wanted {
                        if !wanted.scroll_enabled {
                            lock_scroll(&webview, false);
                        }
                        if wanted.javascript {
                            execute(&webview, &wanted.injected_script);
                        }
                    }
                    report(
                        &app,
                        id,
                        WebViewEvent::Loaded {
                            url: source_of(&webview),
                        },
                    );
                    Ok(())
                }));
            let _ = webview.add_NavigationCompleted(&completed, &mut token);

            let weak = Rc::downgrade(app);
            let message =
                WebMessageReceivedEventHandler::create(Box::new(move |webview, arguments| {
                    let (Some(app), Some(webview), Some(arguments)) =
                        (weak.upgrade(), webview, arguments)
                    else {
                        return Ok(());
                    };
                    let mut text = PWSTR::null();
                    // Only strings are messages; other JSON values are ignored.
                    if arguments.TryGetWebMessageAsString(&mut text).is_ok() {
                        report(
                            &app,
                            id,
                            WebViewEvent::Message {
                                message: take_string(text),
                                url: source_of(&webview),
                            },
                        );
                    }
                    Ok(())
                }));
            let _ = webview.add_WebMessageReceived(&message, &mut token);

            // New windows are never opened.
            let new_window = NewWindowRequestedEventHandler::create(Box::new(|_, arguments| {
                if let Some(arguments) = arguments {
                    arguments.SetHandled(true)?;
                }
                Ok(())
            }));
            let _ = webview.add_NewWindowRequested(&new_window, &mut token);

            // Permission prompts (camera, location, notifications…) are denied.
            let permission = PermissionRequestedEventHandler::create(Box::new(|_, arguments| {
                if let Some(arguments) = arguments {
                    arguments.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            }));
            let _ = webview.add_PermissionRequested(&permission, &mut token);

            // A crashed renderer leaves a blank page: the application is told.
            let weak = Rc::downgrade(app);
            let process_failed = ProcessFailedEventHandler::create(Box::new(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    report(
                        &app,
                        id,
                        WebViewEvent::Failed(String::from(
                            "The web page process ended unexpectedly",
                        )),
                    );
                }
                Ok(())
            }));
            let _ = webview.add_ProcessFailed(&process_failed, &mut token);

            // Keyboard focus inside the page is the view's focus.
            let weak = Rc::downgrade(app);
            let got_focus = FocusChangedEventHandler::create(Box::new(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    crate::focus::native_changed(&app, id, true);
                }
                Ok(())
            }));
            let _ = controller.add_GotFocus(&got_focus, &mut token);
            let weak = Rc::downgrade(app);
            let lost_focus = FocusChangedEventHandler::create(Box::new(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    crate::focus::native_changed(&app, id, false);
                }
                Ok(())
            }));
            let _ = controller.add_LostFocus(&lost_focus, &mut token);
            // Tab past the last (or first) element of the page leaves it.
            let weak = Rc::downgrade(app);
            let move_focus =
                MoveFocusRequestedEventHandler::create(Box::new(move |_, arguments| {
                    let (Some(app), Some(arguments)) = (weak.upgrade(), arguments) else {
                        return Ok(());
                    };
                    let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC;
                    arguments.Reason(&mut reason)?;
                    let forward = reason != COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS;
                    if reason == COREWEBVIEW2_MOVE_FOCUS_REASON_NEXT
                        || reason == COREWEBVIEW2_MOVE_FOCUS_REASON_PREVIOUS
                    {
                        let window = app.with_state(|state| state.scene.window_of(id)).flatten();
                        if let Some(window) = window {
                            arguments.SetHandled(crate::focus::advance(&app, window, forward))?;
                            app.post_pump();
                        }
                    }
                    Ok(())
                }));
            let _ = controller.add_MoveFocusRequested(&move_focus, &mut token);

            // Pages reach the application through `PamNative.postMessage`
            // (main frame only). The script is only guaranteed to run in
            // documents created after its registration completed, so the
            // first navigation waits for that.
            let bridge = format!(
                "if(window.top===window){{{}}}",
                BRIDGE_SCRIPT.replace("__PAM_POST__", "window.chrome.webview.postMessage")
            );
            let weak = Rc::downgrade(app);
            let registered = AddScriptToExecuteOnDocumentCreatedCompletedHandler::create(Box::new(
                move |_, _| {
                    if let Some(app) = weak.upgrade() {
                        synchronize(&app, id);
                    }
                    Ok(())
                },
            ));
            if webview
                .AddScriptToExecuteOnDocumentCreated(&HSTRING::from(bridge), &registered)
                .is_err()
            {
                synchronize(app, id);
            }
        }
    }

    fn error_text(status: COREWEBVIEW2_WEB_ERROR_STATUS) -> String {
        format!("The page could not be loaded (WebView2 error {})", status.0)
    }

    /// Brings the page to the wanted state, comparing with what was applied.
    fn synchronize(app: &Rc<App>, id: NodeId) {
        let plan = app
            .with_state(|state| {
                let host = host_mut(state, id)?;
                let webview = host.webview.clone()?;
                let wanted = host.wanted.clone()?;
                let previous = host.applied.replace(wanted.clone());
                if previous.as_ref().map(|previous| &previous.source) != Some(&wanted.source)
                    && wanted.source != WebSource::None
                {
                    host.loaded = false;
                }
                Some((
                    webview,
                    wanted,
                    previous,
                    host.loaded,
                    host.default_user_agent.clone(),
                ))
            })
            .flatten();
        let Some((webview, wanted, previous, loaded, default_user_agent)) = plan else {
            return;
        };
        // SAFETY: COM calls on the live page; strings outlive the calls.
        unsafe {
            if let Ok(settings) = webview.Settings() {
                let _ = settings.SetIsScriptEnabled(wanted.javascript);
                if let Ok(settings) = settings.cast::<ICoreWebView2Settings2>() {
                    let agent = if wanted.user_agent.is_empty() {
                        &default_user_agent
                    } else {
                        &wanted.user_agent
                    };
                    if !agent.is_empty() {
                        let _ = settings.SetUserAgent(&HSTRING::from(agent.as_str()));
                    }
                }
            }
            if previous
                .as_ref()
                .is_some_and(|previous| previous.stop != wanted.stop)
            {
                let _ = webview.Stop();
            }
            let source_changed =
                previous.as_ref().map(|previous| &previous.source) != Some(&wanted.source);
            if source_changed {
                match &wanted.source {
                    WebSource::None => {
                        if previous.is_some() {
                            let _ = webview.Stop();
                        }
                    }
                    WebSource::Url(url) => {
                        if let Err(error) = webview.Navigate(&HSTRING::from(url.as_str())) {
                            fail(
                                app,
                                id,
                                format!("The page could not be loaded ({})", error.message()),
                            );
                        }
                    }
                    WebSource::Html(html) => {
                        if let Err(error) = webview.NavigateToString(&HSTRING::from(html.as_str()))
                        {
                            fail(
                                app,
                                id,
                                format!("The page could not be loaded ({})", error.message()),
                            );
                        }
                    }
                }
            } else if loaded {
                let script_changed = previous
                    .as_ref()
                    .is_some_and(|previous| previous.injected_script != wanted.injected_script);
                if script_changed && wanted.javascript {
                    execute(&webview, &wanted.injected_script);
                }
                if previous
                    .as_ref()
                    .is_some_and(|previous| previous.scroll_enabled != wanted.scroll_enabled)
                {
                    lock_scroll(&webview, wanted.scroll_enabled);
                }
            }
        }
    }

    /// `Backend::set_web_view`.
    pub fn apply(app: &Rc<App>, id: NodeId, wanted: &WebViewState) {
        let known = app
            .with_state(|state| {
                let host = host_mut(state, id)?;
                let navigates = host.wanted.as_ref().map(|previous| &previous.source)
                    != Some(&wanted.source)
                    && wanted.source != WebSource::None;
                if navigates {
                    // A new page may fail (and be reported) again.
                    host.failed = false;
                }
                host.wanted = Some(wanted.clone());
                let ready = host.webview.is_some();
                let failure = host.failure.clone();
                if !ready {
                    // The page is created when the view is placed in a window.
                    if let Some(window) = state.scene.window_of(id) {
                        state.scene.damage_all(window);
                    }
                }
                Some((ready, navigates, failure))
            })
            .flatten();
        match known {
            Some((true, _, _)) => synchronize(app, id),
            Some((false, true, failure)) => {
                // Without a runtime (or a page) there is nothing to load into.
                if let Some(message) =
                    failure.or_else(|| SHARED.with(|shared| shared.borrow().failure.clone()))
                {
                    fail(app, id, message);
                }
            }
            _ => {}
        }
    }

    extern "system" fn host_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: the container has no behaviour of its own.
        unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
    }

    fn create_container(app: &App, parent: HWND) -> Option<HWND> {
        let registered = SHARED
            .with(|shared| std::mem::replace(&mut shared.borrow_mut().class_registered, true));
        // SAFETY: standard class registration and creation of a child window.
        unsafe {
            if !registered {
                let class = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(host_proc),
                    hInstance: app.instance,
                    lpszClassName: HOST_CLASS,
                    ..Default::default()
                };
                RegisterClassExW(&class);
            }
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                HOST_CLASS,
                PCWSTR::null(),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(app.instance),
                None,
            )
            .ok()
        }
    }

    /// Creates the container and the page when needed and moves them over the node.
    pub fn place(app: &Rc<App>, id: NodeId, placement: &Placement) {
        let plan = app
            .with_state(|state| {
                let parent = state.scene.windows.get(&placement.window)?.hwnd;
                let host = host_mut(state, id)?;
                // A page is only created once something is asked of it.
                host.wanted.as_ref()?;
                let moved = host.placement.replace(*placement) != Some(*placement);
                // The view is now in another top-level window: WebView2
                // cannot follow, so the page starts over there.
                let stale = (host.container.is_some() && host.parent != parent)
                    .then(|| (host.container, host.reset()));
                Some((
                    parent,
                    host.container,
                    host.controller.clone(),
                    moved,
                    stale,
                ))
            })
            .flatten();
        let Some((parent, mut container, controller, moved, stale)) = plan else {
            return;
        };
        if let Some((stale_container, stale_controller)) = stale {
            if let Some(controller) = &stale_controller {
                close(controller);
            }
            if let Some(stale_container) = stale_container {
                // SAFETY: destroying our own child window; no state borrow is held.
                unsafe {
                    let _ = DestroyWindow(stale_container);
                }
            }
        }
        let mut start = false;
        if container.is_none() {
            container = create_container(app, parent);
            let Some(created) = container else {
                return creation_failed(
                    app,
                    id,
                    None,
                    String::from("WebView2 could not create its window"),
                );
            };
            app.with_state(|state| {
                if let Some(host) = host_mut(state, id) {
                    host.container = Some(created);
                    host.parent = parent;
                    host.creating = true;
                    host.failure = None;
                }
            });
            start = true;
        }
        let Some(container) = container else { return };
        if moved || start {
            let rect = pixel_rect_nearest(placement.rect, placement.scale);
            let (width, height) = (
                (rect.right - rect.left).max(0),
                (rect.bottom - rect.top).max(0),
            );
            // Without a page the container would be an unpainted hole over the node.
            let visible = placement.visible && width > 0 && height > 0 && controller.is_some();
            // SAFETY: live windows and COM objects of this thread; these
            // calls dispatch messages and no state borrow is held. The
            // region passed to SetWindowRgn is owned by the system afterwards.
            unsafe {
                if visible {
                    let _ = SetWindowPos(
                        container,
                        None,
                        rect.left,
                        rect.top,
                        width,
                        height,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                    match placement.clip {
                        Some(clip) => {
                            let clip = pixel_rect_nearest(clip, placement.scale);
                            SetWindowRgn(
                                container,
                                Some(CreateRectRgn(
                                    clip.left - rect.left,
                                    clip.top - rect.top,
                                    clip.right - rect.left,
                                    clip.bottom - rect.top,
                                )),
                                true,
                            );
                        }
                        None => {
                            SetWindowRgn(container, None, true);
                        }
                    }
                    let _ = ShowWindow(container, SW_SHOWNA);
                } else {
                    let _ = ShowWindow(container, SW_HIDE);
                }
                if let Some(controller) = &controller {
                    let _ = controller.SetBounds(RECT {
                        left: 0,
                        top: 0,
                        right: width,
                        bottom: height,
                    });
                    let _ = controller.SetIsVisible(visible);
                    let _ = controller.NotifyParentWindowPositionChanged();
                }
            }
        }
        if start {
            create_controller(app, id);
        }
    }

    /// Hides the page of a view that is no longer in a window. `parent` is
    /// the window being synchronized: pages of other windows are left alone.
    pub fn detach(app: &Rc<App>, id: NodeId, parent: HWND) {
        let hidden = app
            .with_state(|state| {
                let host = host_mut(state, id)?;
                if host.parent != parent {
                    return None;
                }
                host.placement.take()?;
                Some((host.container?, host.controller.clone()))
            })
            .flatten();
        if let Some((container, controller)) = hidden {
            // SAFETY: live window and COM object of this thread; no state borrow is held.
            unsafe {
                let _ = ShowWindow(container, SW_HIDE);
                if let Some(controller) = controller {
                    let _ = controller.SetIsVisible(false);
                }
            }
        }
    }

    /// The top-level window died and took the container with it. The page
    /// is closed by `close_retired`, once the state is free.
    pub fn parent_destroyed(host: &mut WebHost, parent: HWND) {
        if host.parent != parent {
            return;
        }
        if let Some(controller) = host.reset() {
            SHARED.with(|shared| shared.borrow_mut().retired.push(controller));
        }
    }

    /// Closes the pages of destroyed windows. Runs outside any state borrow.
    pub fn close_retired() {
        let retired = SHARED.with(|shared| std::mem::take(&mut shared.borrow_mut().retired));
        for controller in &retired {
            close(controller);
        }
    }

    /// The top-level window moved on screen: pages reposition their popups
    /// (select lists, context menus) by it.
    pub fn window_moved(app: &Rc<App>, window: WindowId) {
        let controllers: Vec<ICoreWebView2Controller> = app
            .with_state(|state| {
                state
                    .scene
                    .natives
                    .iter()
                    .filter(|id| state.scene.window_of(**id) == Some(window))
                    .filter_map(|id| state.scene.views.get(id)?.web.as_ref()?.controller.clone())
                    .collect()
            })
            .unwrap_or_default();
        for controller in controllers {
            // SAFETY: COM call on a live controller; no state borrow is held.
            unsafe {
                let _ = controller.NotifyParentWindowPositionChanged();
            }
        }
    }

    /// Moves keyboard focus into the page of `id`. Returns whether the view
    /// is a web view.
    pub fn focus(app: &Rc<App>, id: NodeId) -> bool {
        let Some(Some(controller)) = app.with_state(|state| {
            state
                .scene
                .views
                .get(&id)?
                .web
                .as_ref()
                .map(|host| host.controller.clone())
        }) else {
            return false;
        };
        if let Some(controller) = controller {
            // SAFETY: COM call on a live controller; no state borrow is held.
            unsafe {
                let _ = controller.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON_PROGRAMMATIC);
            }
        }
        true
    }

    /// Releases the page of a destroyed view.
    pub fn release(app: &Rc<App>, host: Box<WebHost>) {
        let id = host.id;
        SHARED.with(|shared| shared.borrow_mut().waiting.retain(|waiting| *waiting != id));
        let (controller, container) = (host.controller.clone(), host.container);
        if let Some(container) = container {
            // SAFETY: hiding a live window; no state borrow is held.
            unsafe {
                let _ = ShowWindow(container, SW_HIDE);
            }
        }
        // Closed from the message loop: the view may be destroyed in answer
        // to an event the page itself is still delivering.
        app.defer(Box::new(move |_| {
            if let Some(controller) = &controller {
                close(controller);
            }
            if let Some(container) = container {
                // SAFETY: destroying our own child window; no state borrow is held.
                unsafe {
                    let _ = DestroyWindow(container);
                }
            }
        }));
    }

    /// Releases the environment before COM is uninitialized.
    pub fn shutdown() {
        let shared = SHARED.with(|shared| std::mem::take(&mut *shared.borrow_mut()));
        for controller in &shared.retired {
            close(controller);
        }
        drop(shared);
    }
}
