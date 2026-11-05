//! The page of a `WebView` view, through WebKitGTK.
//!
//! WebKitGTK is loaded the first time an application shows a `WebView`, not
//! when the host starts: linked in, it costs every application about 17 MB of
//! resident memory and makes `libwebkitgtk-6.0` a hard requirement. The few
//! entry points used here are resolved by name; objects, properties and
//! signals go through GObject.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::rc::Rc;
use std::sync::OnceLock;

use gtk::glib;
use gtk::glib::gobject_ffi::GObject;
use gtk::glib::translate::*;
use gtk::prelude::*;
use pam_desktop_ui::backend::{BackendEvent, NodeId, WebSource, WebViewEvent, WebViewState};
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::webview::{BRIDGE_SCRIPT, navigation_allowed};

use crate::view::PamView;

/// Reported when the library is not installed.
pub const UNAVAILABLE: &str =
    "WebView needs WebKitGTK 6 (libwebkitgtk-6.0.so.4), which is not installed";

const LIBRARY: &CStr = c"libwebkitgtk-6.0.so.4";

// WebKitUserContentInjectedFrames, WebKitUserScriptInjectionTime, WebKitUserStyleLevel.
const INJECT_TOP_FRAME: c_int = 1;
const INJECT_AT_DOCUMENT_START: c_int = 0;
const STYLE_LEVEL_USER: c_int = 0;
// WebKitLoadEvent.
const LOAD_STARTED: c_int = 0;
const LOAD_FINISHED: c_int = 3;
// WebKitPolicyDecisionType.
const POLICY_NAVIGATION_ACTION: c_int = 0;
const POLICY_NEW_WINDOW_ACTION: c_int = 1;
// WebKitNetworkError.
const NETWORK_ERROR_CANCELLED: c_int = 302;

type Strings = *const *const c_char;

/// The entry points of WebKitGTK used by the backend.
struct WebKit {
    web_view_type: glib::Type,
    navigation_decision_type: glib::Type,
    network_error: glib::ffi::GQuark,
    content_manager_new: unsafe extern "C" fn() -> *mut GObject,
    add_script: unsafe extern "C" fn(*mut GObject, *mut c_void),
    register_handler:
        unsafe extern "C" fn(*mut GObject, *const c_char, *const c_char) -> glib::ffi::gboolean,
    add_style_sheet: unsafe extern "C" fn(*mut GObject, *mut c_void),
    remove_all_style_sheets: unsafe extern "C" fn(*mut GObject),
    user_script_new:
        unsafe extern "C" fn(*const c_char, c_int, c_int, Strings, Strings) -> *mut c_void,
    user_script_unref: unsafe extern "C" fn(*mut c_void),
    user_style_sheet_new:
        unsafe extern "C" fn(*const c_char, c_int, c_int, Strings, Strings) -> *mut c_void,
    user_style_sheet_unref: unsafe extern "C" fn(*mut c_void),
    #[allow(clippy::type_complexity)]
    evaluate_javascript: unsafe extern "C" fn(
        *mut GObject,
        *const c_char,
        isize,
        *const c_char,
        *const c_char,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ),
    load_uri: unsafe extern "C" fn(*mut GObject, *const c_char),
    load_html: unsafe extern "C" fn(*mut GObject, *const c_char, *const c_char),
    stop_loading: unsafe extern "C" fn(*mut GObject),
    settings: unsafe extern "C" fn(*mut GObject) -> *mut GObject,
    decision_ignore: unsafe extern "C" fn(*mut GObject),
    decision_action: unsafe extern "C" fn(*mut GObject) -> *mut c_void,
    action_frame_name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    action_request: unsafe extern "C" fn(*mut c_void) -> *mut GObject,
    request_uri: unsafe extern "C" fn(*mut GObject) -> *const c_char,
    permission_deny: unsafe extern "C" fn(*mut GObject),
    value_to_string: unsafe extern "C" fn(*mut GObject) -> *mut c_char,
}

impl WebKit {
    /// The library, loaded on first use; `None` when it is not installed or
    /// lacks an entry point.
    fn get() -> Option<&'static WebKit> {
        static WEBKIT: OnceLock<Option<WebKit>> = OnceLock::new();
        WEBKIT.get_or_init(|| unsafe { Self::load() }).as_ref()
    }

    // Every symbol takes the type of the field or binding it is assigned to.
    #[allow(clippy::missing_transmute_annotations)]
    unsafe fn load() -> Option<WebKit> {
        unsafe {
            // Never unloaded: GObject types cannot be unregistered.
            let library = libc::dlopen(LIBRARY.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
            if library.is_null() {
                return None;
            }
            // Also finds the symbols of the libraries WebKitGTK depends on
            // (`jsc_value_to_string` lives in libjavascriptcoregtk).
            macro_rules! symbol {
                ($name:literal) => {{
                    let pointer = libc::dlsym(library, concat!($name, "\0").as_ptr().cast());
                    if pointer.is_null() {
                        return None;
                    }
                    std::mem::transmute::<*mut c_void, _>(pointer)
                }};
            }
            let web_view_type: unsafe extern "C" fn() -> glib::ffi::GType =
                symbol!("webkit_web_view_get_type");
            let navigation_decision_type: unsafe extern "C" fn() -> glib::ffi::GType =
                symbol!("webkit_navigation_policy_decision_get_type");
            let network_error: unsafe extern "C" fn() -> glib::ffi::GQuark =
                symbol!("webkit_network_error_quark");
            Some(WebKit {
                web_view_type: from_glib(web_view_type()),
                navigation_decision_type: from_glib(navigation_decision_type()),
                network_error: network_error(),
                content_manager_new: symbol!("webkit_user_content_manager_new"),
                add_script: symbol!("webkit_user_content_manager_add_script"),
                register_handler: symbol!(
                    "webkit_user_content_manager_register_script_message_handler"
                ),
                add_style_sheet: symbol!("webkit_user_content_manager_add_style_sheet"),
                remove_all_style_sheets: symbol!(
                    "webkit_user_content_manager_remove_all_style_sheets"
                ),
                user_script_new: symbol!("webkit_user_script_new"),
                user_script_unref: symbol!("webkit_user_script_unref"),
                user_style_sheet_new: symbol!("webkit_user_style_sheet_new"),
                user_style_sheet_unref: symbol!("webkit_user_style_sheet_unref"),
                evaluate_javascript: symbol!("webkit_web_view_evaluate_javascript"),
                load_uri: symbol!("webkit_web_view_load_uri"),
                load_html: symbol!("webkit_web_view_load_html"),
                stop_loading: symbol!("webkit_web_view_stop_loading"),
                settings: symbol!("webkit_web_view_get_settings"),
                decision_ignore: symbol!("webkit_policy_decision_ignore"),
                decision_action: symbol!("webkit_navigation_policy_decision_get_navigation_action"),
                action_frame_name: symbol!("webkit_navigation_action_get_frame_name"),
                action_request: symbol!("webkit_navigation_action_get_request"),
                request_uri: symbol!("webkit_uri_request_get_uri"),
                permission_deny: symbol!("webkit_permission_request_deny"),
                value_to_string: symbol!("jsc_value_to_string"),
            })
        }
    }
}

/// A C string of text that may hold NUL bytes (they end the text).
fn c_string(text: &str) -> CString {
    let end = text.find('\0').unwrap_or(text.len());
    CString::new(&text[..end]).unwrap_or_default()
}

fn raw(object: &glib::Object) -> *mut GObject {
    object.as_ptr()
}

/// The value of an enumeration argument of a signal.
fn enum_argument(value: &glib::Value) -> c_int {
    unsafe { glib::gobject_ffi::g_value_get_enum(value.to_glib_none().0) }
}

/// The settings of a web view (owned by the view).
fn settings_of(webkit: &WebKit, view: &glib::Object) -> Option<glib::Object> {
    unsafe { from_glib_none((webkit.settings)(raw(view))) }
}

fn uri_of(view: &glib::Object) -> String {
    view.property::<Option<String>>("uri").unwrap_or_default()
}

pub struct WebHost {
    webkit: &'static WebKit,
    view: glib::Object,
    widget: gtk::Widget,
    content: glib::Object,
    state: Option<WebViewState>,
    /// Shared with the signal handlers.
    allowed_hosts: Rc<RefCell<Vec<String>>>,
    script: Rc<RefCell<String>>,
    javascript: Rc<Cell<bool>>,
    /// A main-frame page finished and nothing new started since.
    loaded: Rc<Cell<bool>>,
}

fn evaluate(webkit: &WebKit, view: &glib::Object, script: &str) {
    if !script.is_empty() {
        let script = c_string(script);
        unsafe {
            (webkit.evaluate_javascript)(
                raw(view),
                script.as_ptr(),
                -1,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            );
        }
    }
}

impl WebHost {
    /// `None` when WebKitGTK is not installed.
    pub fn new(id: NodeId, container: &PamView, events: &Rc<EventQueue>) -> Option<Self> {
        let webkit = WebKit::get()?;
        let content: glib::Object = unsafe { from_glib_full((webkit.content_manager_new)()) };
        // Pages reach the application through `PamNative.postMessage`.
        let bridge = c_string(&BRIDGE_SCRIPT.replace(
            "__PAM_POST__",
            "window.webkit.messageHandlers.pam.postMessage",
        ));
        unsafe {
            let script = (webkit.user_script_new)(
                bridge.as_ptr(),
                INJECT_TOP_FRAME,
                INJECT_AT_DOCUMENT_START,
                ptr::null(),
                ptr::null(),
            );
            (webkit.add_script)(raw(&content), script);
            (webkit.user_script_unref)(script);
            (webkit.register_handler)(raw(&content), c"pam".as_ptr(), ptr::null());
        }
        let view = glib::Object::with_mut_values(
            webkit.web_view_type,
            &mut [("user-content-manager", content.to_value())],
        );
        let widget = view.clone().downcast::<gtk::Widget>().ok()?;
        if let Some(settings) = settings_of(webkit, &view) {
            settings.set_property("javascript-can-open-windows-automatically", false);
            settings.set_property("enable-developer-extras", false);
        }
        widget.set_parent(container);

        let allowed_hosts = Rc::new(RefCell::new(Vec::new()));
        let script = Rc::new(RefCell::new(String::new()));
        let javascript = Rc::new(Cell::new(true));
        let loaded = Rc::new(Cell::new(false));

        {
            let (events, view) = (Rc::clone(events), view.downgrade());
            content.connect_local("script-message-received::pam", false, move |arguments| {
                let value = arguments.get(1)?.get::<glib::Object>().ok()?;
                let message: glib::GString =
                    unsafe { from_glib_full((webkit.value_to_string)(raw(&value))) };
                let url = view.upgrade().map(|view| uri_of(&view)).unwrap_or_default();
                events.push(BackendEvent::WebView {
                    id,
                    event: WebViewEvent::Message {
                        message: message.to_string(),
                        url,
                    },
                });
                None
            });
        }
        {
            let (events, script, javascript, loaded) = (
                Rc::clone(events),
                Rc::clone(&script),
                Rc::clone(&javascript),
                Rc::clone(&loaded),
            );
            view.connect_local("load-changed", false, move |arguments| {
                let view = arguments.first()?.get::<glib::Object>().ok()?;
                match enum_argument(arguments.get(1)?) {
                    LOAD_STARTED => loaded.set(false),
                    LOAD_FINISHED => {
                        // A failed load also "finishes"; it was reported already.
                        if view.property::<bool>("is-loading")
                            || view.property::<f64>("estimated-load-progress") < 1.0
                        {
                            return None;
                        }
                        loaded.set(true);
                        if javascript.get() {
                            evaluate(webkit, &view, &script.borrow());
                        }
                        events.push(BackendEvent::WebView {
                            id,
                            event: WebViewEvent::Loaded { url: uri_of(&view) },
                        });
                    }
                    _ => {}
                }
                None
            });
        }
        {
            let events = Rc::clone(events);
            view.connect_local("load-failed", false, move |arguments| {
                let error = arguments.get(3)?.get::<glib::Error>().ok()?;
                // A stopped load (timeout, new source) is not a failure of the page.
                let cancelled = unsafe {
                    let error = error.as_ptr();
                    (*error).domain == webkit.network_error
                        && (*error).code == NETWORK_ERROR_CANCELLED
                };
                if !cancelled {
                    events.push(BackendEvent::WebView {
                        id,
                        event: WebViewEvent::Failed(error.message().to_string()),
                    });
                }
                Some(false.to_value())
            });
        }
        {
            let (events, allowed_hosts) = (Rc::clone(events), Rc::clone(&allowed_hosts));
            view.connect_local("decide-policy", false, move |arguments| {
                let decision = arguments.get(1)?.get::<glib::Object>().ok()?;
                let handled = match enum_argument(arguments.get(2)?) {
                    // New windows are never opened.
                    POLICY_NEW_WINDOW_ACTION => {
                        unsafe { (webkit.decision_ignore)(raw(&decision)) };
                        true
                    }
                    POLICY_NAVIGATION_ACTION
                        if decision.type_().is_a(webkit.navigation_decision_type) =>
                    unsafe {
                        let action = (webkit.decision_action)(raw(&decision));
                        if action.is_null() {
                            return Some(false.to_value());
                        }
                        // Only the main frame is restricted.
                        let frame = (webkit.action_frame_name)(action);
                        if !frame.is_null() && !CStr::from_ptr(frame).is_empty() {
                            return Some(false.to_value());
                        }
                        let request = (webkit.action_request)(action);
                        let url = if request.is_null() {
                            String::new()
                        } else {
                            let uri = (webkit.request_uri)(request);
                            if uri.is_null() {
                                String::new()
                            } else {
                                CStr::from_ptr(uri).to_string_lossy().into_owned()
                            }
                        };
                        if navigation_allowed(&allowed_hosts.borrow(), &url) {
                            false
                        } else {
                            (webkit.decision_ignore)(raw(&decision));
                            events.push(BackendEvent::WebView {
                                id,
                                event: WebViewEvent::Blocked { url },
                            });
                            true
                        }
                    },
                    _ => false,
                };
                Some(handled.to_value())
            });
        }
        // Permission prompts (camera, location, notifications…) are denied.
        view.connect_local("permission-request", false, move |arguments| {
            let request = arguments.get(1)?.get::<glib::Object>().ok()?;
            unsafe { (webkit.permission_deny)(raw(&request)) };
            Some(true.to_value())
        });

        Some(Self {
            webkit,
            view,
            widget,
            content,
            state: None,
            allowed_hosts,
            script,
            javascript,
            loaded,
        })
    }

    fn stop(&self) {
        unsafe { (self.webkit.stop_loading)(raw(&self.view)) };
    }

    pub fn destroy(&mut self) {
        self.stop();
        self.widget.unparent();
    }

    pub fn apply(&mut self, state: &WebViewState) {
        let webkit = self.webkit;
        let previous = self.state.take();
        *self.allowed_hosts.borrow_mut() = state.allowed_hosts.clone();
        self.javascript.set(state.javascript);
        if let Some(settings) = settings_of(webkit, &self.view) {
            settings.set_property("enable-javascript", state.javascript);
            settings.set_property("enable-html5-local-storage", state.dom_storage);
            settings.set_property("media-playback-requires-user-gesture", !state.inline_media);
            // No value restores the user agent of WebKitGTK.
            let user_agent = (!state.user_agent.is_empty()).then(|| state.user_agent.clone());
            settings.set_property("user-agent", user_agent);
        }
        let script_changed = previous
            .as_ref()
            .is_some_and(|previous| previous.injected_script != state.injected_script);
        *self.script.borrow_mut() = state.injected_script.clone();
        if previous
            .as_ref()
            .is_some_and(|previous| previous.stop != state.stop)
        {
            self.stop();
        }
        if previous.as_ref().map(|previous| &previous.source) != Some(&state.source) {
            match &state.source {
                WebSource::None => self.stop(),
                WebSource::Url(url) => {
                    self.loaded.set(false);
                    let url = c_string(url);
                    unsafe { (webkit.load_uri)(raw(&self.view), url.as_ptr()) };
                }
                WebSource::Html(html) => {
                    self.loaded.set(false);
                    let html = c_string(html);
                    unsafe { (webkit.load_html)(raw(&self.view), html.as_ptr(), ptr::null()) };
                }
            }
        } else if script_changed && self.loaded.get() && state.javascript {
            evaluate(webkit, &self.view, &state.injected_script);
        }
        if previous.as_ref().map(|previous| previous.scroll_enabled) != Some(state.scroll_enabled) {
            unsafe {
                (webkit.remove_all_style_sheets)(raw(&self.content));
                if !state.scroll_enabled {
                    let sheet = (webkit.user_style_sheet_new)(
                        c"html,body{overflow:hidden !important;}".as_ptr(),
                        INJECT_TOP_FRAME,
                        STYLE_LEVEL_USER,
                        ptr::null(),
                        ptr::null(),
                    );
                    (webkit.add_style_sheet)(raw(&self.content), sheet);
                    (webkit.user_style_sheet_unref)(sheet);
                }
            }
        }
        self.state = Some(state.clone());
    }
}
