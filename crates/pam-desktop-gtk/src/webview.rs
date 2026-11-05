//! The page of a `WebView` view, through WebKitGTK.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use pam_desktop_ui::backend::{BackendEvent, NodeId, WebSource, WebViewEvent, WebViewState};
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::webview::{navigation_allowed, BRIDGE_SCRIPT};
use webkit::prelude::*;

use crate::view::PamView;

pub struct WebHost {
    view: webkit::WebView,
    state: Option<WebViewState>,
    /// Shared with the signal handlers.
    allowed_hosts: Rc<RefCell<Vec<String>>>,
    script: Rc<RefCell<String>>,
    javascript: Rc<Cell<bool>>,
    /// A main-frame page finished and nothing new started since.
    loaded: Rc<Cell<bool>>,
}

fn evaluate(view: &webkit::WebView, script: &str) {
    if !script.is_empty() {
        view.evaluate_javascript(script, None, None, gtk::gio::Cancellable::NONE, |_| {});
    }
}

impl WebHost {
    pub fn new(id: NodeId, container: &PamView, events: &Rc<EventQueue>) -> Self {
        let content = webkit::UserContentManager::new();
        // Pages reach the application through `PamNative.postMessage`.
        let bridge = BRIDGE_SCRIPT.replace("__PAM_POST__", "window.webkit.messageHandlers.pam.postMessage");
        content.add_script(&webkit::UserScript::new(&bridge, webkit::UserContentInjectedFrames::TopFrame, webkit::UserScriptInjectionTime::Start, &[], &[]));
        content.register_script_message_handler("pam", None);
        let view = webkit::WebView::builder().user_content_manager(&content).build();
        if let Some(settings) = webkit::prelude::WebViewExt::settings(&view) {
            settings.set_javascript_can_open_windows_automatically(false);
            settings.set_enable_developer_extras(false);
        }
        view.set_parent(container);

        let allowed_hosts = Rc::new(RefCell::new(Vec::new()));
        let script = Rc::new(RefCell::new(String::new()));
        let javascript = Rc::new(Cell::new(true));
        let loaded = Rc::new(Cell::new(false));

        {
            let (events, view) = (Rc::clone(events), view.downgrade());
            content.connect_script_message_received(Some("pam"), move |_, value| {
                let url = view.upgrade().and_then(|view| view.uri()).map(|uri| uri.to_string()).unwrap_or_default();
                events.push(BackendEvent::WebView { id, event: WebViewEvent::Message { message: value.to_str().to_string(), url } });
            });
        }
        {
            let (events, script, javascript, loaded) = (Rc::clone(events), Rc::clone(&script), Rc::clone(&javascript), Rc::clone(&loaded));
            view.connect_load_changed(move |view, event| match event {
                webkit::LoadEvent::Started => loaded.set(false),
                webkit::LoadEvent::Finished => {
                    // A failed load also "finishes"; it was reported already.
                    if view.is_loading() || view.estimated_load_progress() < 1.0 {
                        return;
                    }
                    loaded.set(true);
                    if javascript.get() {
                        evaluate(view, &script.borrow());
                    }
                    let url = view.uri().map(|uri| uri.to_string()).unwrap_or_default();
                    events.push(BackendEvent::WebView { id, event: WebViewEvent::Loaded { url } });
                }
                _ => {}
            });
        }
        {
            let events = Rc::clone(events);
            view.connect_load_failed(move |_, _, _, error| {
                // A stopped load (timeout, new source) is not a failure of the page.
                if !error.matches(webkit::NetworkError::Cancelled) {
                    events.push(BackendEvent::WebView { id, event: WebViewEvent::Failed(error.message().to_string()) });
                }
                false
            });
        }
        {
            let (events, allowed_hosts) = (Rc::clone(events), Rc::clone(&allowed_hosts));
            view.connect_decide_policy(move |_, decision, kind| {
                match kind {
                    // New windows are never opened.
                    webkit::PolicyDecisionType::NewWindowAction => {
                        decision.ignore();
                        true
                    }
                    webkit::PolicyDecisionType::NavigationAction => {
                        let Some(navigation) = decision.downcast_ref::<webkit::NavigationPolicyDecision>() else { return false };
                        let Some(action) = navigation.navigation_action() else { return false };
                        // Only the main frame is restricted.
                        if action.frame_name().is_some_and(|name| !name.is_empty()) {
                            return false;
                        }
                        let url = action.request().and_then(|request| request.uri()).map(|uri| uri.to_string()).unwrap_or_default();
                        if navigation_allowed(&allowed_hosts.borrow(), &url) {
                            return false;
                        }
                        decision.ignore();
                        events.push(BackendEvent::WebView { id, event: WebViewEvent::Blocked { url } });
                        true
                    }
                    _ => false,
                }
            });
        }
        // Permission prompts (camera, location, notifications…) are denied.
        view.connect_permission_request(|_, request| {
            request.deny();
            true
        });

        Self { view, state: None, allowed_hosts, script, javascript, loaded }
    }

    pub fn destroy(&mut self) {
        self.view.stop_loading();
        self.view.unparent();
    }

    pub fn apply(&mut self, state: &WebViewState) {
        let previous = self.state.take();
        *self.allowed_hosts.borrow_mut() = state.allowed_hosts.clone();
        self.javascript.set(state.javascript);
        if let Some(settings) = webkit::prelude::WebViewExt::settings(&self.view) {
            settings.set_enable_javascript(state.javascript);
            settings.set_enable_html5_local_storage(state.dom_storage);
            settings.set_media_playback_requires_user_gesture(!state.inline_media);
            if state.user_agent.is_empty() {
                settings.set_user_agent(None);
            } else {
                settings.set_user_agent(Some(&state.user_agent));
            }
        }
        let script_changed = previous.as_ref().is_some_and(|previous| previous.injected_script != state.injected_script);
        *self.script.borrow_mut() = state.injected_script.clone();
        if previous.as_ref().is_some_and(|previous| previous.stop != state.stop) {
            self.view.stop_loading();
        }
        if previous.as_ref().map(|previous| &previous.source) != Some(&state.source) {
            match &state.source {
                WebSource::None => self.view.stop_loading(),
                WebSource::Url(url) => {
                    self.loaded.set(false);
                    self.view.load_uri(url);
                }
                WebSource::Html(html) => {
                    self.loaded.set(false);
                    self.view.load_html(html, None);
                }
            }
        } else if script_changed && self.loaded.get() && state.javascript {
            evaluate(&self.view, &state.injected_script);
        }
        if previous.as_ref().map(|previous| previous.scroll_enabled) != Some(state.scroll_enabled) {
            let overflow = if state.scroll_enabled { "" } else { "html,body{overflow:hidden !important;}" };
            if let Some(content) = self.view.user_content_manager() {
                content.remove_all_style_sheets();
                if !overflow.is_empty() {
                    content.add_style_sheet(&webkit::UserStyleSheet::new(overflow, webkit::UserContentInjectedFrames::TopFrame, webkit::UserStyleLevel::User, &[], &[]));
                }
            }
        }
        self.state = Some(state.clone());
    }
}
