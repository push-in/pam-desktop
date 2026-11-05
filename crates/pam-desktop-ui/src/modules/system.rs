//! Small platform modules that need nothing but the standard library:
//! device status, permissions, deep links, cache accounting, background
//! tokens and the toast fallback.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::{request_map, Module, Reply};
use crate::wiremap::WireMap;

// ---- device ---------------------------------------------------------------

const NETWORK_NONE: i64 = 1;
const NETWORK_WIFI: i64 = 2;
const NETWORK_ETHERNET: i64 = 4;

/// Battery and connectivity as the OS reports them right now.
#[must_use]
pub fn device_status() -> WireMap {
    let (level, charging) = battery();
    WireMap::new()
        .with("batteryLevel", level)
        .with("charging", charging)
        .with("networkType", network_type())
        .with("expensiveNetwork", false)
        .with("lowPowerMode", false)
}

/// Charge in `0..=1` (`-1` when unknown) and whether it is charging.
fn battery() -> (f64, bool) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = fs::read_dir("/sys/class/power_supply") {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                let kind = fs::read_to_string(path.join("type")).unwrap_or_default();
                if kind.trim() != "Battery" {
                    continue;
                }
                let Some(capacity) = fs::read_to_string(path.join("capacity")).ok().and_then(|value| value.trim().parse::<f64>().ok())
                else {
                    continue;
                };
                let status = fs::read_to_string(path.join("status")).unwrap_or_default();
                return ((capacity / 100.0).clamp(0.0, 1.0), matches!(status.trim(), "Charging" | "Full"));
            }
        }
    }
    (-1.0, false)
}

fn network_type() -> i64 {
    #[cfg(target_os = "linux")]
    {
        let mut connected = NETWORK_NONE;
        if let Ok(entries) = fs::read_dir("/sys/class/net") {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if entry.file_name() == "lo" {
                    continue;
                }
                let up = fs::read_to_string(path.join("operstate")).is_ok_and(|state| state.trim() == "up");
                if !up {
                    continue;
                }
                if path.join("wireless").exists() {
                    connected = connected.max(NETWORK_WIFI);
                } else {
                    return NETWORK_ETHERNET;
                }
            }
        }
        connected
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Desktops are assumed to be on a wired or equivalent connection.
        NETWORK_ETHERNET
    }
}

struct Subscription {
    interval: Duration,
    /// The status last handed to PHP (`None` before the first read).
    delivered: Option<Vec<u8>>,
    waiter: Option<Reply>,
}

#[derive(Default)]
struct DeviceState {
    subscriptions: HashMap<i64, Subscription>,
    next: i64,
    polling: bool,
}

/// `device`: status snapshots and a change-driven watch (a re-armed long poll).
#[derive(Default)]
pub struct Device {
    state: Arc<Mutex<DeviceState>>,
}

impl Device {
    fn ensure_polling(&self) {
        let Ok(mut state) = self.state.lock() else { return };
        if state.polling {
            return;
        }
        state.polling = true;
        let shared = Arc::clone(&self.state);
        let _ = thread::Builder::new().name("pam-device".into()).spawn(move || loop {
            let interval = {
                let Ok(mut state) = shared.lock() else { return };
                if state.subscriptions.is_empty() {
                    state.polling = false;
                    return;
                }
                let status = device_status().encode();
                for subscription in state.subscriptions.values_mut() {
                    if subscription.delivered.as_deref() != Some(status.as_slice()) {
                        if let Some(waiter) = subscription.waiter.take() {
                            subscription.delivered = Some(status.clone());
                            waiter.success(status.clone());
                        }
                    }
                }
                state.subscriptions.values().map(|subscription| subscription.interval).min().unwrap_or(Duration::from_secs(1))
            };
            thread::sleep(interval);
        });
    }
}

impl Module for Device {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "status" => reply.map(&device_status()),
            "watch" => {
                let interval = request.integer("intervalMs").unwrap_or(1000).clamp(250, 60_000) as u64;
                let id = {
                    let Ok(mut state) = self.state.lock() else { return reply.failure("Device status failed") };
                    state.next += 1;
                    let id = state.next;
                    state.subscriptions.insert(id, Subscription { interval: Duration::from_millis(interval), delivered: None, waiter: None });
                    id
                };
                self.ensure_polling();
                reply.map(&WireMap::new().with("subscription", id));
            }
            "next" => {
                let Some(id) = request.integer("subscription") else { return reply.failure("Missing device subscription") };
                let Ok(mut state) = self.state.lock() else { return reply.failure("Device status failed") };
                let Some(subscription) = state.subscriptions.get_mut(&id) else { return reply.failure("Unknown device subscription") };
                if subscription.waiter.is_some() {
                    return reply.failure("Observation already has a pending read");
                }
                let status = device_status().encode();
                if subscription.delivered.as_deref() == Some(status.as_slice()) {
                    // Nothing new: park until the status changes.
                    subscription.waiter = Some(reply);
                } else {
                    subscription.delivered = Some(status.clone());
                    reply.success(status);
                }
            }
            "stop" => {
                let Some(id) = request.integer("subscription") else { return reply.failure("Missing device subscription") };
                let stopped = self.state.lock().ok().and_then(|mut state| state.subscriptions.remove(&id));
                if let Some(waiter) = stopped.and_then(|subscription| subscription.waiter) {
                    waiter.failure("Observation stopped");
                }
                reply.done();
            }
            other => reply.failure(format!("Unknown device method {other}")),
        }
    }

    fn reset(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.subscriptions.clear();
        }
    }
}

// ---- permissions ----------------------------------------------------------

/// `permissions`: a desktop process already holds what it can use, so every
/// kind reports as granted and nothing ever needs a rationale.
#[derive(Default)]
pub struct Permissions;

impl Module for Permissions {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "status" | "request" => match request.integer("kind") {
                Some(kind) if (1..=9).contains(&kind) => reply.map(&WireMap::new().with("status", 1_i64).with("canAskAgain", false)),
                Some(kind) => reply.failure(format!("Unknown permission kind {kind}")),
                None => reply.failure("Missing permission kind"),
            },
            "shouldShowRationale" => reply.map(&WireMap::new().with("show", false)),
            "openSettings" => reply.done(),
            other => reply.failure(format!("Unknown permissions method {other}")),
        }
    }
}

// ---- linking --------------------------------------------------------------

const MAX_PENDING_LINKS: usize = 32;
const MAX_LINK_BYTES: usize = 8192;

#[derive(Default)]
struct LinkState {
    initial: Option<String>,
    pending: VecDeque<String>,
    waiter: Option<Reply>,
}

/// `linking`: the launch URL and links that arrive while the app runs.
#[derive(Clone, Default)]
pub struct Linking {
    state: Arc<Mutex<LinkState>>,
}

impl Linking {
    #[must_use]
    pub fn new(initial: Option<String>) -> Self {
        Self { state: Arc::new(Mutex::new(LinkState { initial: initial.filter(|url| !url.trim().is_empty()), ..LinkState::default() })) }
    }

    /// A link arrived (a second launch forwarded its URL, a scheme activation).
    pub fn deliver(&self, url: &str) {
        let url = url.trim();
        if url.is_empty() || url.len() > MAX_LINK_BYTES {
            return;
        }
        let Ok(mut state) = self.state.lock() else { return };
        match state.waiter.take() {
            Some(waiter) => waiter.map(&WireMap::new().with("url", url)),
            None => {
                if state.pending.len() == MAX_PENDING_LINKS {
                    state.pending.pop_front();
                }
                state.pending.push_back(url.to_owned());
            }
        }
    }
}

impl Module for Linking {
    fn call(&mut self, method: &str, _payload: &[u8], reply: Reply) {
        let Ok(mut state) = self.state.lock() else { return reply.failure("Linking failed") };
        match method {
            // Consumed by the first read.
            "initialUrl" => reply.map(&WireMap::new().with("url", state.initial.take().unwrap_or_default())),
            "nextUrl" => {
                if let Some(url) = state.pending.pop_front() {
                    reply.map(&WireMap::new().with("url", url));
                } else if state.waiter.is_some() {
                    reply.failure("Only one deep-link listener can wait at a time");
                } else {
                    state.waiter = Some(reply);
                }
            }
            other => reply.failure(format!("Unknown linking method {other}")),
        }
    }

    fn reset(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            // The parked listener belongs to the previous worker.
            state.waiter = None;
        }
    }
}

/// Schemes an application may hand to the system.
#[must_use]
pub fn can_open_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else { return false };
    !rest.is_empty() && matches!(scheme.to_ascii_lowercase().as_str(), "https" | "http" | "mailto" | "tel" | "geo")
}

/// Opens a URL with the system's default handler.
pub fn open_url(url: &str) -> Result<(), String> {
    if !can_open_url(url) {
        return Err("Unsupported URL scheme".into());
    }
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    } else if cfg!(windows) {
        // `rundll32` avoids the shell: the URL is never parsed as a command line.
        let mut command = std::process::Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|_| String::from("Cannot open URL"))
}

// ---- cache ----------------------------------------------------------------

pub const IMAGE_CACHE_DIRECTORY: &str = "pam-images-v1";
pub const MEDIA_CACHE_DIRECTORY: &str = "pam-media-v1";
const SHARES_DIRECTORY: &str = "pam-incoming-shares";

fn directory_usage(directory: &Path) -> (i64, i64) {
    let mut files = 0_i64;
    let mut bytes = 0_i64;
    let mut stack = vec![directory.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.filter_map(Result::ok) {
            match entry.metadata() {
                Ok(metadata) if metadata.is_dir() => stack.push(entry.path()),
                Ok(metadata) => {
                    files += 1;
                    bytes += i64::try_from(metadata.len()).unwrap_or(i64::MAX);
                }
                Err(_) => {}
            }
        }
    }
    (files, bytes)
}

/// `cache`: how much the image and media caches hold, and clearing them.
pub struct Cache {
    root: PathBuf,
}

impl Cache {
    #[must_use]
    pub fn new(cache_directory: &Path) -> Self {
        Self { root: cache_directory.to_path_buf() }
    }

    fn usage(&self, freed: i64) -> WireMap {
        let (image_files, image_bytes) = directory_usage(&self.root.join(IMAGE_CACHE_DIRECTORY));
        let (media_files, media_bytes) = directory_usage(&self.root.join(MEDIA_CACHE_DIRECTORY));
        let (share_files, share_bytes) = directory_usage(&self.root.join(SHARES_DIRECTORY));
        WireMap::new()
            .with("fileCount", image_files + media_files + share_files)
            .with("imageBytes", image_bytes)
            .with("mediaBytes", media_bytes)
            .with("temporaryBytes", share_bytes)
            .with("totalBytes", image_bytes + media_bytes + share_bytes)
            .with("freedBytes", freed.max(0))
    }

    fn total(&self) -> i64 {
        [IMAGE_CACHE_DIRECTORY, MEDIA_CACHE_DIRECTORY, SHARES_DIRECTORY].iter().map(|name| directory_usage(&self.root.join(name)).1).sum()
    }

    fn clear(&self, preserve_offline: bool) {
        let _ = fs::remove_dir_all(self.root.join(IMAGE_CACHE_DIRECTORY));
        let _ = fs::remove_dir_all(self.root.join(SHARES_DIRECTORY));
        let media = self.root.join(MEDIA_CACHE_DIRECTORY);
        if !preserve_offline {
            let _ = fs::remove_dir_all(media);
            return;
        }
        // Media pinned for offline use (`<name>.media` with a `<name>.pin`) stays.
        let Ok(entries) = fs::read_dir(&media) else { return };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let pinned = path.with_extension("pin").exists() || path.extension().is_some_and(|extension| extension == "pin");
            if !pinned {
                let _ = if path.is_dir() { fs::remove_dir_all(&path) } else { fs::remove_file(&path) };
            }
        }
    }
}

impl Module for Cache {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "usage" => reply.map(&self.usage(0)),
            "clear" => {
                let before = self.total();
                self.clear(request.boolean("preserveOffline").unwrap_or(true));
                reply.map(&self.usage(before - self.total()));
            }
            other => reply.failure(format!("Unknown cache method {other}")),
        }
    }
}

// ---- background -----------------------------------------------------------

/// `background`: keep-alive tokens. A desktop process is never suspended, so
/// a token only needs to exist.
#[derive(Default)]
pub struct Background {
    next: i64,
}

impl Module for Background {
    fn call(&mut self, method: &str, payload: &[u8], reply: Reply) {
        let request = match request_map(payload) {
            Ok(request) => request,
            Err(error) => return reply.failure(error),
        };
        match method {
            "begin" => {
                self.next += 1;
                reply.map(&WireMap::new().with("token", self.next));
            }
            "end" => match request.integer("token") {
                Some(_) => reply.done(),
                None => reply.failure("Missing background token"),
            },
            other => reply.failure(format!("Unknown background method {other}")),
        }
    }
}

// ---- toast ----------------------------------------------------------------

/// `toast`: the plugin's module name. Without the plugin's host there is
/// nothing to draw, but the call must not fail an application flow.
#[derive(Default)]
pub struct Toast;

impl Module for Toast {
    fn call(&mut self, _method: &str, _payload: &[u8], reply: Reply) {
        reply.done();
    }
}

// ---- environment ----------------------------------------------------------

/// The BCP-47 tag of the process locale (`pt-BR`, `en-US`).
#[must_use]
pub fn locale_tag() -> String {
    let raw = crate::input_format::system_locale();
    let tag = raw.split(['.', '@']).next().unwrap_or("en-US").replace('_', "-");
    if tag.is_empty() { String::from("en-US") } else { tag }
}

/// The IANA time zone id of the system (`America/Sao_Paulo`), `UTC` when unknown.
#[must_use]
pub fn time_zone() -> String {
    if let Ok(zone) = std::env::var("TZ") {
        let zone = zone.trim_start_matches(':');
        if !zone.is_empty() && !zone.starts_with('/') {
            return zone.to_owned();
        }
    }
    if let Ok(target) = fs::read_link("/etc/localtime") {
        let text = target.to_string_lossy();
        if let Some(position) = text.find("zoneinfo/") {
            return text[position + "zoneinfo/".len()..].to_owned();
        }
    }
    if let Ok(zone) = fs::read_to_string("/etc/timezone") {
        let zone = zone.trim();
        if !zone.is_empty() {
            return zone.to_owned();
        }
    }
    String::from("UTC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_safe_schemes_open() {
        assert!(can_open_url("https://pam.dev/docs"));
        assert!(can_open_url("mailto:hi@example.com"));
        assert!(!can_open_url("file:///etc/passwd"));
        assert!(!can_open_url("javascript:alert(1)"));
        assert!(!can_open_url("https:"));
        assert!(!can_open_url("no scheme"));
    }

    #[test]
    fn cache_usage_counts_and_clear_keeps_pinned_media() {
        let root = std::env::temp_dir().join(format!("pam-cache-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(IMAGE_CACHE_DIRECTORY)).unwrap();
        fs::create_dir_all(root.join(MEDIA_CACHE_DIRECTORY)).unwrap();
        fs::write(root.join(IMAGE_CACHE_DIRECTORY).join("a.bin"), [0_u8; 10]).unwrap();
        fs::write(root.join(MEDIA_CACHE_DIRECTORY).join("song.media"), [0_u8; 20]).unwrap();
        fs::write(root.join(MEDIA_CACHE_DIRECTORY).join("song.pin"), []).unwrap();
        fs::write(root.join(MEDIA_CACHE_DIRECTORY).join("clip.media"), [0_u8; 30]).unwrap();
        let cache = Cache::new(&root);
        assert_eq!(cache.usage(0).integer("totalBytes"), Some(60));
        assert_eq!(cache.usage(0).integer("fileCount"), Some(4));
        cache.clear(true);
        assert_eq!(cache.usage(0).integer("imageBytes"), Some(0));
        assert_eq!(cache.usage(0).integer("mediaBytes"), Some(20));
        cache.clear(false);
        assert_eq!(cache.total(), 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn locale_is_a_bcp47_tag() {
        let tag = locale_tag();
        assert!(!tag.contains('_') && !tag.contains('.') && !tag.is_empty());
    }
}
