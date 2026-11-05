//! Web views: what page a `WebView` node shows, the navigation policy, the
//! load timeout and the page bridge events.
//!
//! The backend owns the platform web engine. It receives a [`WebViewState`]
//! and reports page loads, failures and messages posted by the page.

use pam_desktop_wire::{NodeKind, PropKey};

use crate::animator::Timer;
use crate::backend::{NodeId, WebSource, WebViewEvent, WebViewState};
use crate::context::{event_kind, Ctx, HostCommand};
use crate::scene::{Change, IdMap};
use crate::wiremap::WireMap;

/// A page that neither finished nor failed after this long is stopped.
const LOAD_TIMEOUT_MS: u64 = 30_000;
/// Longest message a page may post.
pub const MAX_MESSAGE_CHARS: usize = 1_048_576;

/// The script every backend installs at document start (main frame only):
/// it gives pages `window.PamNative.postMessage(text)` and the
/// `ReactNativeWebView` alias. `__PAM_POST__` is replaced by the
/// expression that hands a string to the platform's message handler.
pub const BRIDGE_SCRIPT: &str = "(function(){if(window.PamNative)return;var post=function(value){__PAM_POST__(String(value));};var bridge=Object.freeze({postMessage:post});window.PamNative=bridge;window.ReactNativeWebView=bridge;})();";

/// Whether a main-frame navigation to `url` is allowed by the host list.
/// An empty list allows everything; `about:` and `data:` always pass.
#[must_use]
pub fn navigation_allowed(allowed_hosts: &[String], url: &str) -> bool {
    if allowed_hosts.is_empty() {
        return true;
    }
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("about:") || lower.starts_with("data:") {
        return true;
    }
    host_of(&lower).is_some_and(|host| allowed_hosts.iter().any(|allowed| *allowed == host))
}

/// The lower-cased host of a URL, without credentials or port.
#[must_use]
pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, host)| host);
    let host = if let Some(bracketed) = host.strip_prefix('[') { bracketed.split(']').next()? } else { host.split(':').next()? };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The message of a navigation the policy refused.
#[must_use]
pub fn blocked_message(url: &str) -> String {
    format!("WebView blocked navigation to {}", host_of(url).unwrap_or_else(|| String::from("unknown host")))
}

#[derive(Default)]
struct Page {
    pushed: Option<WebViewState>,
    stop: u32,
    /// The URL the main frame shows, as last reported.
    url: String,
    dirty: bool,
}

#[derive(Default)]
pub struct WebViews {
    pages: IdMap<Page>,
    dirty: bool,
}

impl WebViews {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::WebView) => {
                self.pages.insert(id, Page { dirty: true, ..Page::default() });
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::WebView) => {
                self.pages.remove(&id);
                ctx.timers.cancel(Timer::WebViewTimeout(id));
            }
            Change::Updated(id, _) => {
                if let Some(page) = self.pages.get_mut(&id) {
                    page.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let dirty: Vec<NodeId> = self.pages.iter().filter(|(_, page)| page.dirty).map(|(id, _)| *id).collect();
        for id in dirty {
            self.sync(ctx, id);
        }
    }

    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let authored = props.text_or_empty(PropKey::WebViewSource);
        let source = if authored.trim().is_empty() {
            WebSource::None
        } else if authored.trim_start().starts_with('<') {
            WebSource::Html(authored.to_owned())
        } else {
            WebSource::Url(authored.trim().to_owned())
        };
        let allowed_hosts: Vec<String> = props.text_or_empty(PropKey::WebViewAllowedHosts).split('\n').map(|host| host.trim().to_ascii_lowercase()).filter(|host| !host.is_empty()).collect();
        let Some(page) = self.pages.get_mut(&id) else { return };
        page.dirty = false;
        let mut state = WebViewState {
            source,
            stop: page.stop,
            javascript: props.flag_or(PropKey::WebViewJavaScriptEnabled, true),
            dom_storage: props.flag_or(PropKey::WebViewDomStorageEnabled, true),
            user_agent: props.text_or_empty(PropKey::WebViewUserAgent).to_owned(),
            injected_script: props.text_or_empty(PropKey::WebViewInjectedJavaScript).to_owned(),
            inline_media: props.flag_or(PropKey::WebViewAllowsInlineMedia, true),
            allowed_hosts,
            scroll_enabled: props.flag_or(PropKey::ScrollEnabled, true),
        };
        let previous = page.pushed.as_ref();
        if previous == Some(&state) {
            return;
        }
        let source_changed = previous.map(|previous| &previous.source) != Some(&state.source);
        let hosts_changed = previous.is_some_and(|previous| previous.allowed_hosts != state.allowed_hosts);
        if source_changed {
            page.url.clear();
            match &state.source {
                WebSource::None => {
                    ctx.timers.cancel(Timer::WebViewTimeout(id));
                }
                WebSource::Url(url) if !navigation_allowed(&state.allowed_hosts, url) => {
                    // Refused before it starts: the page on screen stays.
                    ctx.timers.cancel(Timer::WebViewTimeout(id));
                    let message = blocked_message(url);
                    ctx.emit_if(id, PropKey::OnWebViewError, event_kind::WEBVIEW_ERROR, WireMap::new().with("message", message.as_str()).encode());
                    state.source = previous.map_or(WebSource::None, |previous| previous.source.clone());
                }
                _ => ctx.timers.schedule(Timer::WebViewTimeout(id), ctx.now_ms + LOAD_TIMEOUT_MS),
            }
        } else if hosts_changed && !page.url.is_empty() && !navigation_allowed(&state.allowed_hosts, &page.url) {
            // The page on screen is no longer allowed: stop whatever loads.
            page.stop = page.stop.wrapping_add(1);
            state.stop = page.stop;
            ctx.timers.cancel(Timer::WebViewTimeout(id));
            let message = "WebView navigation was blocked by the allowed-host policy";
            ctx.emit_if(id, PropKey::OnWebViewError, event_kind::WEBVIEW_ERROR, WireMap::new().with("message", message).encode());
        }
        ctx.commands.push(HostCommand::SetWebView { id, state: Box::new(state.clone()) });
        page.pushed = Some(state);
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(page) = self.pages.get_mut(&id) else { return };
        let Some(mut state) = page.pushed.clone() else { return };
        page.stop = page.stop.wrapping_add(1);
        state.stop = page.stop;
        ctx.commands.push(HostCommand::SetWebView { id, state: Box::new(state.clone()) });
        page.pushed = Some(state);
        let message = "WebView navigation timed out after 30 seconds";
        ctx.emit_if(id, PropKey::OnWebViewError, event_kind::WEBVIEW_ERROR, WireMap::new().with("message", message).encode());
    }

    pub fn on_event(&mut self, ctx: &mut Ctx<'_>, id: NodeId, event: WebViewEvent) {
        let Some(page) = self.pages.get_mut(&id) else { return };
        match event {
            WebViewEvent::Loaded { url } => {
                page.url = url;
                ctx.timers.cancel(Timer::WebViewTimeout(id));
                ctx.emit_if(id, PropKey::OnWebViewLoad, event_kind::WEBVIEW_LOAD, Vec::new());
            }
            WebViewEvent::Failed(message) => {
                ctx.timers.cancel(Timer::WebViewTimeout(id));
                ctx.emit_if(id, PropKey::OnWebViewError, event_kind::WEBVIEW_ERROR, WireMap::new().with("message", message.as_str()).encode());
            }
            WebViewEvent::Blocked { url } => {
                ctx.timers.cancel(Timer::WebViewTimeout(id));
                let message = blocked_message(&url);
                ctx.emit_if(id, PropKey::OnWebViewError, event_kind::WEBVIEW_ERROR, WireMap::new().with("message", message.as_str()).encode());
            }
            WebViewEvent::Message { message, url } => {
                if message.chars().count() <= MAX_MESSAGE_CHARS {
                    let payload = WireMap::new().with("message", message.as_str()).with("url", url.as_str()).encode();
                    ctx.emit_if(id, PropKey::OnWebViewMessage, event_kind::WEBVIEW_MESSAGE, payload);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_extracted_without_credentials_or_port() {
        assert_eq!(host_of("https://Example.com/path"), Some("example.com".into()));
        assert_eq!(host_of("https://user:pass@example.com:8443/x?y#z"), Some("example.com".into()));
        assert_eq!(host_of("http://[::1]:8080/"), Some("::1".into()));
        assert_eq!(host_of("mailto:someone@example.com"), None);
        assert_eq!(host_of("https:///nohost"), None);
    }

    #[test]
    fn the_allow_list_matches_hosts_exactly() {
        let hosts = vec![String::from("example.com")];
        assert!(navigation_allowed(&[], "https://anything.test/"));
        assert!(navigation_allowed(&hosts, "https://example.com/page"));
        assert!(navigation_allowed(&hosts, "about:blank"));
        assert!(navigation_allowed(&hosts, "data:text/html,hi"));
        assert!(!navigation_allowed(&hosts, "https://sub.example.com/"));
        assert!(!navigation_allowed(&hosts, "https://evil.test/?example.com"));
        assert!(!navigation_allowed(&hosts, "mailto:a@example.com"));
        assert_eq!(blocked_message("https://evil.test/x"), "WebView blocked navigation to evil.test");
        assert_eq!(blocked_message("javascript:void(0)"), "WebView blocked navigation to unknown host");
    }
}
