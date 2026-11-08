//! The page of a `WebView` view, through `WKWebView`.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSView, NSWindowOrderingMode};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_foundation::{NSError, NSString, NSURL, NSURLRequest};
use objc2_web_kit::{
    WKAudiovisualMediaTypes, WKFrameInfo, WKMediaCaptureType, WKNavigation, WKNavigationAction,
    WKNavigationActionPolicy, WKNavigationDelegate, WKPermissionDecision, WKScriptMessage,
    WKScriptMessageHandler, WKSecurityOrigin, WKUIDelegate, WKUserContentController, WKUserScript,
    WKUserScriptInjectionTime, WKWebView, WKWebViewConfiguration, WKWebpagePreferences,
    WKWebsiteDataStore, WKWindowFeatures,
};
use pam_desktop_ui::backend::{BackendEvent, NodeId, WebSource, WebViewEvent, WebViewState};
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::webview::{BRIDGE_SCRIPT, navigation_allowed};

use crate::events::Events;
use crate::util::{cg_size, ns_string};

/// Name of the message handler pages post to.
const HANDLER: &str = "pam";
/// Adds or removes the rule that stops the page from scrolling.
const LOCK_SCROLL: &str = "(function(){if(document.getElementById('pam-no-scroll'))return;var style=document.createElement('style');style.id='pam-no-scroll';style.textContent='html,body{overflow:hidden !important;}';(document.head||document.documentElement).appendChild(style);})();";
const UNLOCK_SCROLL: &str =
    "(function(){var style=document.getElementById('pam-no-scroll');if(style)style.remove();})();";

/// State shared with the delegate of the web view.
struct Shared {
    id: NodeId,
    events: Rc<Events>,
    allowed_hosts: RefCell<Vec<String>>,
    /// Evaluated after every page finish.
    script: RefCell<String>,
    javascript: Cell<bool>,
    /// A main-frame page finished and nothing new started since.
    loaded: Cell<bool>,
}

impl Shared {
    fn emit(&self, event: WebViewEvent) {
        self.events
            .push(BackendEvent::WebView { id: self.id, event });
    }

    fn failed(&self, error: &NSError) {
        let domain = error.domain().to_string();
        // A stopped load (timeout, new source, refused navigation) is not
        // a failure of the page.
        let cancelled = (domain == "NSURLErrorDomain" && error.code() == -999)
            || (domain == "WebKitErrorDomain" && error.code() == 102);
        if !cancelled {
            self.emit(WebViewEvent::Failed(
                error.localizedDescription().to_string(),
            ));
        }
    }
}

fn evaluate(view: &WKWebView, script: &str) {
    if !script.is_empty() {
        // SAFETY: plain call on the main thread; no completion handler.
        unsafe { view.evaluateJavaScript_completionHandler(&ns_string(script), None) };
    }
}

fn address(view: &WKWebView) -> String {
    // SAFETY: plain property read on the main thread.
    unsafe { view.URL() }
        .and_then(|url| url.absoluteString())
        .map(|url| url.to_string())
        .unwrap_or_default()
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only,
    // like every WebKit delegate.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Shared>]
    struct WebDelegate;

    impl WebDelegate {
        /// Permission prompts (camera, microphone) are denied. Outside the
        /// protocol block: the method joined `WKUIDelegate` in macOS 12,
        /// and debug builds refuse to register a protocol method the
        /// running system does not declare.
        #[unsafe(method(webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:))]
        fn request_media_capture(
            &self,
            _view: &WKWebView,
            _origin: &WKSecurityOrigin,
            _frame: &WKFrameInfo,
            _kind: WKMediaCaptureType,
            decision: &block2::DynBlock<dyn Fn(WKPermissionDecision)>,
        ) {
            decision.call((WKPermissionDecision::Deny,));
        }
    }

    unsafe impl NSObjectProtocol for WebDelegate {}

    unsafe impl WKNavigationDelegate for WebDelegate {
        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_policy(
            &self,
            _view: &WKWebView,
            action: &WKNavigationAction,
            preferences: &WKWebpagePreferences,
            decision: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy, NonNull<WKWebpagePreferences>)>,
        ) {
            let shared = self.ivars();
            // SAFETY: plain property accesses on the main thread.
            let policy = unsafe {
                preferences.setAllowsContentJavaScript(shared.javascript.get());
                match action.targetFrame() {
                    // New windows are never opened.
                    None => WKNavigationActionPolicy::Cancel,
                    // Only the main frame is restricted.
                    Some(frame) if frame.isMainFrame() => {
                        let url = action.request().URL().and_then(|url| url.absoluteString()).map(|url| url.to_string()).unwrap_or_default();
                        if navigation_allowed(&shared.allowed_hosts.borrow(), &url) {
                            WKNavigationActionPolicy::Allow
                        } else {
                            shared.emit(WebViewEvent::Blocked { url });
                            WKNavigationActionPolicy::Cancel
                        }
                    }
                    Some(_) => WKNavigationActionPolicy::Allow,
                }
            };
            decision.call((policy, NonNull::from(preferences)));
        }

        #[unsafe(method(webView:didStartProvisionalNavigation:))]
        fn did_start(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>) {
            self.ivars().loaded.set(false);
        }

        #[unsafe(method(webView:didFinishNavigation:))]
        fn did_finish(&self, view: &WKWebView, _navigation: Option<&WKNavigation>) {
            let shared = self.ivars();
            shared.loaded.set(true);
            if shared.javascript.get() {
                let script = shared.script.borrow().clone();
                evaluate(view, &script);
            }
            shared.emit(WebViewEvent::Loaded { url: address(view) });
        }

        #[unsafe(method(webView:didFailNavigation:withError:))]
        fn did_fail(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.ivars().failed(error);
        }

        #[unsafe(method(webView:didFailProvisionalNavigation:withError:))]
        fn did_fail_provisional(&self, _view: &WKWebView, _navigation: Option<&WKNavigation>, error: &NSError) {
            self.ivars().failed(error);
        }

        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn process_did_terminate(&self, _view: &WKWebView) {
            let shared = self.ivars();
            shared.loaded.set(false);
            shared.emit(WebViewEvent::Failed(String::from("The page stopped responding")));
        }
    }

    unsafe impl WKUIDelegate for WebDelegate {
        /// New windows are never opened.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _view: &WKWebView,
            _configuration: &WKWebViewConfiguration,
            _action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            None
        }
    }

    unsafe impl WKScriptMessageHandler for WebDelegate {
        /// The page called `PamNative.postMessage(text)`.
        #[unsafe(method(userContentController:didReceiveScriptMessage:))]
        fn did_receive_message(&self, _controller: &WKUserContentController, message: &WKScriptMessage) {
            // SAFETY: plain property reads on the main thread.
            let (body, view) = unsafe { (message.body(), message.webView()) };
            let Ok(text) = body.downcast::<NSString>() else { return };
            let url = view.map(|view| address(&view)).unwrap_or_default();
            self.ivars().emit(WebViewEvent::Message { message: text.to_string(), url });
        }
    }
);

pub struct WebHost {
    mtm: MainThreadMarker,
    shared: Rc<Shared>,
    delegate: Retained<WebDelegate>,
    view: Option<Retained<WKWebView>>,
    /// What the page was last asked to be.
    state: Option<WebViewState>,
    /// The page last loaded, to load it again in a rebuilt view.
    current: WebSource,
    size: Size,
}

impl WebHost {
    pub fn new(mtm: MainThreadMarker, id: NodeId, events: &Rc<Events>) -> Self {
        let shared = Rc::new(Shared {
            id,
            events: Rc::clone(events),
            allowed_hosts: RefCell::new(Vec::new()),
            script: RefCell::new(String::new()),
            javascript: Cell::new(true),
            loaded: Cell::new(false),
        });
        let this = WebDelegate::alloc(mtm).set_ivars(Rc::clone(&shared));
        // SAFETY: `init` is the designated initializer of `NSObject`.
        let delegate: Retained<WebDelegate> = unsafe { msg_send![super(this), init] };
        Self {
            mtm,
            shared,
            delegate,
            view: None,
            state: None,
            current: WebSource::None,
            size: Size::ZERO,
        }
    }

    /// The view placed inside the node's view, once there is one.
    pub fn view(&self) -> Option<Retained<NSView>> {
        self.view
            .as_ref()
            .map(|view| Retained::into_super(view.clone()))
    }

    /// The scripts every page gets: the bridge, and the scroll lock.
    fn install_scripts(&self, controller: &WKUserContentController, scroll_enabled: bool) {
        // Pages reach the application through `PamNative.postMessage`.
        let bridge = BRIDGE_SCRIPT.replace(
            "__PAM_POST__",
            "window.webkit.messageHandlers.pam.postMessage",
        );
        // SAFETY: plain constructors and calls on the main thread.
        unsafe {
            controller.removeAllUserScripts();
            let script = |source: &str, time: WKUserScriptInjectionTime| {
                WKUserScript::initWithSource_injectionTime_forMainFrameOnly(
                    WKUserScript::alloc(self.mtm),
                    &ns_string(source),
                    time,
                    true,
                )
            };
            controller.addUserScript(&script(&bridge, WKUserScriptInjectionTime::AtDocumentStart));
            if !scroll_enabled {
                controller.addUserScript(&script(
                    LOCK_SCROLL,
                    WKUserScriptInjectionTime::AtDocumentEnd,
                ));
            }
        }
    }

    /// Storage and media policy are fixed when a web view is created.
    fn create(&mut self, state: &WebViewState, parent: &NSView) {
        self.remove();
        // SAFETY: plain constructors and property writes on the main thread;
        // the delegate implements the three protocols it is registered for
        // and is released from them in `remove`.
        let view = unsafe {
            let configuration = WKWebViewConfiguration::new(self.mtm);
            let controller = configuration.userContentController();
            self.install_scripts(&controller, state.scroll_enabled);
            controller.addScriptMessageHandler_name(
                ProtocolObject::from_ref(&*self.delegate),
                &ns_string(HANDLER),
            );
            // Without storage nothing the page keeps outlives the view.
            let store = if state.dom_storage {
                WKWebsiteDataStore::defaultDataStore(self.mtm)
            } else {
                WKWebsiteDataStore::nonPersistentDataStore(self.mtm)
            };
            configuration.setWebsiteDataStore(&store);
            configuration.setMediaTypesRequiringUserActionForPlayback(if state.inline_media {
                WKAudiovisualMediaTypes::None
            } else {
                WKAudiovisualMediaTypes::All
            });
            configuration
                .preferences()
                .setJavaScriptCanOpenWindowsAutomatically(false);
            let frame = CGRect::new(CGPoint::ZERO, cg_size(self.size));
            let view = WKWebView::initWithFrame_configuration(
                WKWebView::alloc(self.mtm),
                frame,
                &configuration,
            );
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*self.delegate)));
            view
        };
        // The page sits under the node's children.
        parent.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
        self.shared.loaded.set(false);
        self.view = Some(view);
    }

    fn remove(&mut self) {
        let Some(view) = self.view.take() else { return };
        // SAFETY: plain calls on the main thread; clears what `create` set.
        unsafe {
            view.stopLoading();
            view.setNavigationDelegate(None);
            view.setUIDelegate(None);
            view.configuration()
                .userContentController()
                .removeScriptMessageHandlerForName(&ns_string(HANDLER));
        }
        view.removeFromSuperview();
    }

    /// The page covers the node's box.
    pub fn layout(&mut self, size: Size) {
        self.size = size;
        if let Some(view) = &self.view {
            view.setFrame(CGRect::new(CGPoint::ZERO, cg_size(size)));
        }
    }

    pub fn apply(&mut self, state: &WebViewState, parent: &NSView) {
        let previous = self.state.take();
        let shared = Rc::clone(&self.shared);
        *shared.allowed_hosts.borrow_mut() = state.allowed_hosts.clone();
        shared.javascript.set(state.javascript);
        let script_changed = previous
            .as_ref()
            .is_some_and(|previous| previous.injected_script != state.injected_script);
        *shared.script.borrow_mut() = state.injected_script.clone();

        let rebuilt = self.view.is_none()
            || previous.as_ref().is_some_and(|previous| {
                previous.dom_storage != state.dom_storage
                    || previous.inline_media != state.inline_media
            });
        if rebuilt {
            self.create(state, parent);
        }
        let Some(view) = self.view.clone() else {
            return;
        };
        // SAFETY: plain property write on the main thread.
        unsafe {
            view.setCustomUserAgent(
                (!state.user_agent.is_empty())
                    .then(|| ns_string(&state.user_agent))
                    .as_deref(),
            )
        };
        if previous
            .as_ref()
            .is_some_and(|previous| previous.stop != state.stop)
        {
            // SAFETY: plain call on the main thread.
            unsafe { view.stopLoading() };
        }
        if !rebuilt
            && previous.as_ref().map(|previous| previous.scroll_enabled)
                != Some(state.scroll_enabled)
        {
            // SAFETY: plain property read on the main thread.
            let controller = unsafe { view.configuration().userContentController() };
            self.install_scripts(&controller, state.scroll_enabled);
            if shared.loaded.get() && state.javascript {
                evaluate(
                    &view,
                    if state.scroll_enabled {
                        UNLOCK_SCROLL
                    } else {
                        LOCK_SCROLL
                    },
                );
            }
        }
        let source_changed =
            previous.as_ref().map(|previous| &previous.source) != Some(&state.source);
        let source = if source_changed {
            Some(state.source.clone())
        } else if rebuilt {
            // A rebuilt view shows nothing: the page it had is loaded again.
            Some(self.current.clone())
        } else {
            None
        };
        if let Some(source) = source.as_ref().filter(|source| **source != WebSource::None) {
            self.current = source.clone();
        }
        match source {
            // SAFETY: plain calls on the main thread.
            Some(WebSource::None) => unsafe { view.stopLoading() },
            Some(WebSource::Url(url)) => {
                shared.loaded.set(false);
                match NSURL::URLWithString(&ns_string(url.trim())) {
                    // SAFETY: plain call on the main thread.
                    Some(url) => {
                        drop(unsafe { view.loadRequest(&NSURLRequest::requestWithURL(&url)) })
                    }
                    None => shared.emit(WebViewEvent::Failed(String::from(
                        "The address is not valid",
                    ))),
                }
            }
            Some(WebSource::Html(html)) => {
                shared.loaded.set(false);
                // SAFETY: plain call on the main thread; no base address,
                // so the page has the `about:blank` origin.
                drop(unsafe { view.loadHTMLString_baseURL(&ns_string(&html), None) });
            }
            None => {
                if script_changed && shared.loaded.get() && state.javascript {
                    evaluate(&view, &state.injected_script);
                }
            }
        }
        self.state = Some(state.clone());
    }

    pub fn destroy(&mut self) {
        self.remove();
    }
}
