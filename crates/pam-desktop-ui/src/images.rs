//! Images: resolves the source of `Image` and `ImageBackground` nodes,
//! downloads remote ones into the disk cache, shares decoded images between
//! nodes and reports the load events.
//!
//! A decoded image is identified by an [`ImageKey`] and owned by the backend.
//! Nodes showing the same source at a similar size share one key; a key
//! nobody shows stays decoded for a while (a bounded least-recently-used
//! set), so scrolling a list back does not decode again.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use pam_desktop_wire::{NodeKind, PropKey};

use crate::animator::{Curve, Easing, Owner};
use crate::backend::{ImageContent, ImageFit, ImageKey, ImageSource, NodeId};
use crate::context::{event_kind, Ctx, HostCommand};
use crate::geometry::{Insets, Size};
use crate::modules::fetch::HttpClient;
use crate::modules::support::{uuid, WorkerPool};
use crate::modules::{files, image};
use crate::props::Props;
use crate::runtime::{Inbound, Inbox};
use crate::scene::{Change, IdMap};
use crate::wiremap::WireMap;

/// Decoded images nobody shows that stay in memory.
const IDLE_LIMIT: usize = 48;
/// Decoded bytes (RGBA) the idle set may hold.
const IDLE_BYTES: u64 = 96 * 1024 * 1024;
/// Largest `data:` URI decoded.
const MAX_DATA_URI_BYTES: usize = 8 * 1024 * 1024;
/// Decode sizes: an image is decoded no larger than the first bucket that
/// covers the box it is shown in, so thumbnails of large photos stay small.
const BUCKETS: [u32; 5] = [128, 256, 512, 1024, 2048];
const DEFAULT_FADE_MS: i64 = 300;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(50);

/// Where sources resolve and how remote ones are fetched.
pub struct ImageEnvironment {
    /// `<cache dir>/pam-images-v1`.
    pub cache_root: PathBuf,
    /// The sandbox `pam-file:///` URIs resolve in.
    pub files_root: PathBuf,
    /// The application bundle, where `asset://` and plain paths resolve.
    pub assets: PathBuf,
    pub client: Option<Arc<dyn HttpClient>>,
    pub inbox: Arc<Inbox>,
    /// Development: plain `http://` sources are allowed.
    pub allow_insecure: bool,
}

/// Results of the download threads, delivered through the inbox.
#[derive(Debug)]
pub enum ImageInbound {
    Progress { key: ImageKey, loaded: u64, total: u64 },
    Fetched { key: ImageKey, result: Result<ImageSource, String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CachePolicy {
    Default,
    Reload,
    ForceCache,
    OnlyIfCached,
    None,
}

impl CachePolicy {
    fn from_props(props: Props<'_>) -> Self {
        match props.integer_or(PropKey::ImageCachePolicy, 1) {
            2 => Self::Reload,
            3 => Self::ForceCache,
            4 => Self::OnlyIfCached,
            5 => Self::None,
            _ => Self::Default,
        }
    }
}

/// What a source string points at.
#[derive(Debug, PartialEq)]
enum Location {
    Bytes(Arc<[u8]>),
    File(PathBuf),
    Remote,
    Invalid(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    Main,
    Placeholder,
}

#[derive(Debug)]
enum EntryState {
    Fetching,
    Decoding,
    Ready(Size),
}

struct Entry {
    /// Key of the index: decode bucket and URI.
    id: String,
    uri: String,
    state: EntryState,
    remote: bool,
    cancel: Arc<AtomicBool>,
    users: Vec<(NodeId, Slot)>,
}

impl Entry {
    fn decoded_bytes(&self) -> u64 {
        match self.state {
            EntryState::Ready(size) => (size.width.max(0.0) as u64) * (size.height.max(0.0) as u64) * 4,
            _ => 0,
        }
    }
}

#[derive(Default)]
struct NodeImage {
    /// Resolved source last requested (after the source set).
    source: String,
    policy: Option<CachePolicy>,
    headers: String,
    bucket: u32,
    main: Option<ImageKey>,
    /// The image of the previous source, kept on screen until the new one arrives.
    previous: Option<ImageKey>,
    placeholder_source: String,
    placeholder: Option<ImageKey>,
    /// Opacity of the main image while it fades in.
    fade: f32,
    pushed: Option<ImageContent>,
    /// A freehand canvas: its image is always aspect-fit and its strokes
    /// follow the picture's rectangle.
    sketch: bool,
    dirty: bool,
}

pub struct Images {
    environment: ImageEnvironment,
    nodes: IdMap<NodeImage>,
    entries: HashMap<ImageKey, Entry>,
    index: HashMap<String, ImageKey>,
    /// Ready entries without users, oldest first.
    idle: VecDeque<ImageKey>,
    next_key: ImageKey,
    pool: WorkerPool,
    /// Sources that failed before any request was made.
    rejected: Vec<(ImageKey, String)>,
    /// Sketch nodes whose picture changed size since the last drain.
    natural_sizes: Vec<(NodeId, Option<Size>)>,
    dirty: bool,
}

fn is_image(kind: NodeKind) -> bool {
    matches!(kind, NodeKind::Image | NodeKind::ImageBackground | NodeKind::DrawingCanvas)
}

fn fit(props: Props<'_>) -> ImageFit {
    match props.integer_or(PropKey::ImageFit, 1) {
        2 => ImageFit::Contain,
        3 => ImageFit::Fill,
        4 => ImageFit::Center,
        5 => ImageFit::Repeat,
        _ => ImageFit::Cover,
    }
}

/// Picks the candidate of a source set (`src 2x, src 640w`) that best fits
/// the screen scale or the view width; `None` when no candidate is valid.
fn pick_from_source_set(set: &str, scale: f32, width: f32) -> Option<&str> {
    let mut best: Option<(f32, &str)> = None;
    for candidate in set.split(',') {
        let candidate = candidate.trim();
        let Some((source, descriptor)) = candidate.rsplit_once(char::is_whitespace) else { continue };
        let source = source.trim();
        let score = if let Some(density) = descriptor.strip_suffix('x').and_then(|number| number.parse::<f32>().ok()) {
            (density - scale).abs()
        } else if let Some(pixels) = descriptor.strip_suffix('w').and_then(|number| number.parse::<f32>().ok()) {
            (pixels - width).abs() / width.max(1.0)
        } else {
            continue;
        };
        if source.is_empty() || !score.is_finite() {
            continue;
        }
        if best.is_none_or(|(current, _)| score < current) {
            best = Some((score, source));
        }
    }
    best.map(|(_, source)| source)
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = text.get(index + 1..index + 3)?;
            decoded.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

fn data_uri(uri: &str) -> Location {
    if uri.len() > MAX_DATA_URI_BYTES {
        return Location::Invalid("Image data URI is too large.");
    }
    let Some((header, data)) = uri[5..].split_once(',') else { return Location::Invalid("Image data URI is malformed.") };
    let mut parts = header.split(';');
    let mime = parts.next().unwrap_or_default();
    if !mime.get(..6).is_some_and(|prefix| prefix.eq_ignore_ascii_case("image/")) {
        return Location::Invalid("Only image data URIs can be displayed.");
    }
    let base64 = parts.any(|part| part.eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        let compact: String = data.chars().filter(|character| !character.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(compact.trim_end_matches('='))
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(compact.trim_end_matches('=')))
            .ok()
    } else {
        percent_decode(data)
    };
    match bytes {
        Some(bytes) if !bytes.is_empty() => Location::Bytes(Arc::from(bytes)),
        _ => Location::Invalid("Image data URI is malformed."),
    }
}

/// A bundle-relative path: no empty, `.` or `..` segments, no `\`, NUL, `?` or `#`.
fn asset_path(root: &Path, relative: &str) -> Option<PathBuf> {
    let relative = relative.trim_start_matches('/');
    if relative.is_empty() || relative.contains(['\\', '\0', '?', '#']) {
        return None;
    }
    let mut path = root.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return None;
        }
        path.push(segment);
    }
    Some(path)
}

fn is_absolute_path(source: &str) -> bool {
    let bytes = source.as_bytes();
    source.starts_with('/') || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'))
}

fn scheme(source: &str) -> Option<&str> {
    let (scheme, _) = source.split_once("://")?;
    (!scheme.is_empty() && scheme.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))).then_some(scheme)
}

fn bucket_for(pixels: f32) -> u32 {
    BUCKETS.into_iter().find(|bucket| *bucket as f32 >= pixels).unwrap_or(0)
}

impl Images {
    #[must_use]
    pub fn new(environment: ImageEnvironment) -> Self {
        Self {
            environment,
            nodes: IdMap::default(),
            entries: HashMap::new(),
            index: HashMap::new(),
            idle: VecDeque::new(),
            next_key: 1,
            pool: WorkerPool::new("pam-image", 4),
            rejected: Vec::new(),
            natural_sizes: Vec::new(),
            dirty: false,
        }
    }

    fn locate(&self, source: &str) -> Location {
        if source.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:")) {
            return data_uri(source);
        }
        match scheme(source).map(str::to_ascii_lowercase).as_deref() {
            Some("https") => Location::Remote,
            Some("http") if self.environment.allow_insecure => Location::Remote,
            Some("http") => Location::Invalid("Remote images require HTTPS."),
            Some("asset") => match asset_path(&self.environment.assets, &source[8..]) {
                Some(path) => Location::File(path),
                None => Location::Invalid("Image asset path is invalid."),
            },
            Some("pam-file") => match files::resolve_uri(&self.environment.files_root, source) {
                Some(path) => Location::File(path),
                None => Location::Invalid("Image file URI is invalid."),
            },
            Some("file") => {
                let path = source[7..].strip_prefix("localhost").unwrap_or(&source[7..]);
                match percent_decode(path).and_then(|bytes| String::from_utf8(bytes).ok()) {
                    // `file:///C:/x.png` names a drive on Windows.
                    Some(path) if path.len() > 3 && is_absolute_path(&path[1..]) && cfg!(windows) => Location::File(PathBuf::from(&path[1..])),
                    Some(path) if path.starts_with('/') => Location::File(PathBuf::from(path)),
                    _ => Location::Invalid("Image file URI is invalid."),
                }
            }
            Some(_) => Location::Invalid("Unsupported image URI scheme."),
            // Desktop applications hand out absolute paths (file dialogs, drops).
            None if is_absolute_path(source) && Path::new(source).is_file() => Location::File(PathBuf::from(source)),
            None => match asset_path(&self.environment.assets, source) {
                Some(path) => Location::File(path),
                None => Location::Invalid("Image asset path is invalid."),
            },
        }
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, kind) if is_image(kind) => {
                self.nodes.insert(id, NodeImage { fade: 1.0, dirty: true, sketch: kind == NodeKind::DrawingCanvas, ..NodeImage::default() });
                self.dirty = true;
            }
            Change::Removed(id, kind) if is_image(kind) => {
                if let Some(node) = self.nodes.remove(&id) {
                    for key in [node.main, node.previous, node.placeholder].into_iter().flatten() {
                        self.release(ctx, key, id);
                    }
                }
            }
            Change::Updated(id, _) | Change::Frame(id) => {
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.dirty = true;
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
        let dirty: Vec<NodeId> = self.nodes.iter().filter(|(_, node)| node.dirty).map(|(id, _)| *id).collect();
        for id in dirty {
            self.sync(ctx, id);
        }
        for (key, message) in std::mem::take(&mut self.rejected) {
            self.failed(ctx, key, &message);
        }
    }

    /// Brings the image of a node in line with its properties and size.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(scene_node) = ctx.scene.node(id) else { return };
        let props = scene_node.props();
        let size = scene_node.frame.size();
        let scale = ctx.scene.environment.scale.max(0.1);
        let authored = props.text_or_empty(PropKey::Source);
        let source = props
            .text(PropKey::ImageSourceSet)
            .and_then(|set| pick_from_source_set(set, scale, size.width))
            .unwrap_or(authored)
            .trim()
            .to_owned();
        let policy = CachePolicy::from_props(props);
        let headers = props.text_or_empty(PropKey::ImageRequestHeaders).to_owned();
        let placeholder_source = props
            .text(PropKey::ImageLoadingIndicatorSource)
            .or_else(|| props.text(PropKey::ImageDefaultSource))
            .unwrap_or_default()
            .trim()
            .to_owned();
        // Tiled and unscaled images need every pixel; the others are decoded
        // no larger than the box needs.
        let multiplier = props.number_or(PropKey::ImageResizeMultiplier, 1.0).clamp(0.1, 8.0);
        let bucket = match fit(props) {
            ImageFit::Center | ImageFit::Repeat => 0,
            _ => bucket_for(size.width.max(size.height) * scale * multiplier),
        };
        let wants_progress = scene_node.handles(PropKey::OnImageProgress);

        let Some(node) = self.nodes.get_mut(&id) else { return };
        node.dirty = false;
        if source.is_empty() {
            let released: Vec<ImageKey> = [node.main.take(), node.previous.take(), node.placeholder.take()].into_iter().flatten().collect();
            node.source.clear();
            node.placeholder_source.clear();
            for key in released {
                self.release(ctx, key, id);
            }
            return self.present(ctx, id);
        }
        // A view without a size has nothing to decode for yet.
        let is_data = source.get(..5).is_some_and(|prefix| prefix.eq_ignore_ascii_case("data:"));
        if size.is_empty() && !is_data {
            node.dirty = true;
            return;
        }
        let grows = bucket != node.bucket && (bucket == 0 || (node.bucket != 0 && bucket > node.bucket));
        let changed = source != node.source || Some(policy) != node.policy || headers != node.headers || (grows && node.main.is_some());
        if changed {
            let old_main = node.main.take();
            let old_previous = node.previous.take();
            node.source.clone_from(&source);
            node.policy = Some(policy);
            node.headers = headers.clone();
            node.bucket = bucket;
            node.fade = 1.0;
            ctx.animator.cancel(Owner::Image(id), ctx.now_ms);
            // The picture on screen stays until the new one is decoded.
            let keep = old_main.filter(|key| self.entries.get(key).is_some_and(|entry| matches!(entry.state, EntryState::Ready(_))));
            for key in [old_previous, old_main.filter(|_| keep.is_none())].into_iter().flatten() {
                self.release(ctx, key, id);
            }
            if let Some(node) = self.nodes.get_mut(&id) {
                node.previous = keep;
            }
            ctx.emit_if(id, PropKey::OnImageLoadStart, event_kind::IMAGE_LOAD_START, Vec::new());
            let key = self.acquire(ctx, id, Slot::Main, &source, bucket, policy, &headers, wants_progress);
            if let Some(node) = self.nodes.get_mut(&id) {
                node.main = Some(key);
            }
            if let Some(EntryState::Ready(size)) = self.entries.get(&key).map(|entry| &entry.state) {
                let size = *size;
                self.shown(ctx, id, key, size, false);
            }
        }
        // The placeholder only matters while the main image is on its way.
        let Some(node) = self.nodes.get_mut(&id) else { return };
        let main_ready = node.main.is_some_and(|key| self.entries.get(&key).is_some_and(|entry| matches!(entry.state, EntryState::Ready(_))));
        let wanted_placeholder = if main_ready || node.previous.is_some() { String::new() } else { placeholder_source };
        if wanted_placeholder != node.placeholder_source {
            let old = node.placeholder.take();
            node.placeholder_source.clone_from(&wanted_placeholder);
            if let Some(key) = old {
                self.release(ctx, key, id);
            }
            if !wanted_placeholder.is_empty() {
                let key = self.acquire(ctx, id, Slot::Placeholder, &wanted_placeholder, bucket, CachePolicy::Default, "", false);
                if let Some(node) = self.nodes.get_mut(&id) {
                    node.placeholder = Some(key);
                }
            }
        }
        self.present(ctx, id);
    }

    /// Finds or starts the decoded image of `source` and registers `node` as a user.
    #[allow(clippy::too_many_arguments)]
    fn acquire(&mut self, ctx: &mut Ctx<'_>, node: NodeId, slot: Slot, source: &str, bucket: u32, policy: CachePolicy, headers: &str, wants_progress: bool) -> ImageKey {
        // Uncached and reloaded images are never shared with other nodes.
        let shared = matches!(policy, CachePolicy::Default | CachePolicy::ForceCache | CachePolicy::OnlyIfCached);
        let id = if shared { format!("{bucket}|{source}") } else { format!("{bucket}|{source}|{}", uuid()) };
        if let Some(key) = self.index.get(&id).copied() {
            if let Some(entry) = self.entries.get_mut(&key) {
                entry.users.push((node, slot));
                self.idle.retain(|idle| *idle != key);
                return key;
            }
        }
        let key = self.next_key;
        self.next_key += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        let location = self.locate(source);
        let max_dimension = (bucket != 0).then_some(bucket);
        let (state, remote) = match location {
            Location::Bytes(bytes) => {
                ctx.commands.push(HostCommand::LoadImage { key, source: ImageSource::Bytes(bytes), max_dimension });
                (EntryState::Decoding, false)
            }
            Location::File(path) => {
                ctx.commands.push(HostCommand::LoadImage { key, source: ImageSource::File(path), max_dimension });
                (EntryState::Decoding, false)
            }
            Location::Remote => {
                self.fetch(key, source, policy, headers, wants_progress, &cancel);
                (EntryState::Fetching, true)
            }
            Location::Invalid(message) => {
                self.rejected.push((key, message.to_owned()));
                (EntryState::Fetching, false)
            }
        };
        self.index.insert(id.clone(), key);
        self.entries.insert(key, Entry { id, uri: source.to_owned(), state, remote, cancel, users: vec![(node, slot)] });
        key
    }

    /// Downloads a remote image on the pool and reports through the inbox.
    fn fetch(&self, key: ImageKey, url: &str, policy: CachePolicy, headers: &str, wants_progress: bool, cancel: &Arc<AtomicBool>) {
        let inbox = Arc::clone(&self.environment.inbox);
        let Some(client) = self.environment.client.clone() else {
            inbox.post(Inbound::Image(ImageInbound::Fetched { key, result: Err("Network images are unavailable.".into()) }));
            return;
        };
        let root = self.environment.cache_root.clone();
        let url = url.to_owned();
        let headers = image::parse_request_headers(headers);
        let cancel = Arc::clone(cancel);
        self.pool.run(move || {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let mut last_report: Option<Instant> = None;
            let mut report = |loaded: u64, total: u64| {
                if wants_progress && last_report.is_none_or(|at| at.elapsed() >= PROGRESS_INTERVAL) {
                    last_report = Some(Instant::now());
                    inbox.post(Inbound::Image(ImageInbound::Progress { key, loaded, total }));
                }
            };
            let cached = image::cache_path(&root, &url);
            let result = match policy {
                CachePolicy::OnlyIfCached if !cached.is_file() => Err(String::from("Image is not available in the local cache.")),
                CachePolicy::OnlyIfCached => Ok(ImageSource::File(cached)),
                CachePolicy::None => {
                    // Fetched under a private name and never kept on disk.
                    let private = format!("pam-uncached-{}", uuid());
                    image::fetch_to_cache_reporting(client.as_ref(), &root, &url, &private, &headers, &cancel, &mut report).and_then(|path| {
                        let bytes = std::fs::read(&path).map_err(|error| error.to_string());
                        let _ = std::fs::remove_file(&path);
                        bytes.map(|bytes| ImageSource::Bytes(Arc::from(bytes)))
                    })
                }
                CachePolicy::Reload => {
                    let _ = std::fs::remove_file(&cached);
                    image::fetch_to_cache_reporting(client.as_ref(), &root, &url, "", &headers, &cancel, &mut report).map(ImageSource::File)
                }
                CachePolicy::Default | CachePolicy::ForceCache => {
                    image::fetch_to_cache_reporting(client.as_ref(), &root, &url, "", &headers, &cancel, &mut report).map(ImageSource::File)
                }
            };
            if !cancel.load(Ordering::Relaxed) {
                inbox.post(Inbound::Image(ImageInbound::Fetched { key, result }));
            }
        });
    }

    /// `node` stops using `key`; an image without users goes idle or away.
    fn release(&mut self, ctx: &mut Ctx<'_>, key: ImageKey, node: NodeId) {
        let Some(entry) = self.entries.get_mut(&key) else { return };
        if let Some(position) = entry.users.iter().position(|(user, _)| *user == node) {
            entry.users.swap_remove(position);
        }
        if !entry.users.is_empty() {
            return;
        }
        match entry.state {
            EntryState::Ready(_) => {
                self.idle.push_back(key);
                self.trim(ctx);
            }
            EntryState::Fetching => {
                entry.cancel.store(true, Ordering::Relaxed);
                self.forget(key);
            }
            // The backend is decoding: the result arrives and goes idle.
            EntryState::Decoding => {}
        }
    }

    fn forget(&mut self, key: ImageKey) {
        if let Some(entry) = self.entries.remove(&key) {
            if self.index.get(&entry.id) == Some(&key) {
                self.index.remove(&entry.id);
            }
        }
        self.idle.retain(|idle| *idle != key);
    }

    /// Drops the oldest idle images beyond the count and memory limits.
    fn trim(&mut self, ctx: &mut Ctx<'_>) {
        let mut bytes: u64 = self.idle.iter().filter_map(|key| self.entries.get(key)).map(Entry::decoded_bytes).sum();
        while self.idle.len() > IDLE_LIMIT || (bytes > IDLE_BYTES && self.idle.len() > 1) {
            let Some(key) = self.idle.pop_front() else { break };
            bytes = bytes.saturating_sub(self.entries.get(&key).map_or(0, Entry::decoded_bytes));
            self.forget(key);
            ctx.commands.push(HostCommand::ReleaseImage(key));
        }
    }

    /// Memory is short: every image nobody shows goes away.
    pub fn purge_idle(&mut self, ctx: &mut Ctx<'_>) {
        for key in std::mem::take(&mut self.idle) {
            self.forget(key);
            ctx.commands.push(HostCommand::ReleaseImage(key));
        }
    }

    pub fn on_inbound(&mut self, ctx: &mut Ctx<'_>, message: ImageInbound) {
        match message {
            ImageInbound::Progress { key, loaded, total } => {
                let Some(entry) = self.entries.get(&key) else { return };
                let (loaded, total) = (i64::try_from(loaded).unwrap_or(i64::MAX), i64::try_from(total).unwrap_or(0));
                for (node, slot) in entry.users.clone() {
                    if slot == Slot::Main {
                        let payload = WireMap::new().with("loaded", loaded).with("total", total).encode();
                        ctx.emit_if(node, PropKey::OnImageProgress, event_kind::IMAGE_PROGRESS, payload);
                    }
                }
            }
            ImageInbound::Fetched { key, result } => {
                let Some(entry) = self.entries.get_mut(&key) else { return };
                match result {
                    Ok(source) => {
                        entry.state = EntryState::Decoding;
                        let max_dimension = entry.id.split('|').next().and_then(|bucket| bucket.parse::<u32>().ok()).filter(|bucket| *bucket != 0);
                        ctx.commands.push(HostCommand::LoadImage { key, source, max_dimension });
                    }
                    Err(message) => self.failed(ctx, key, &message),
                }
            }
        }
    }

    /// The backend decoded `key`.
    pub fn loaded(&mut self, ctx: &mut Ctx<'_>, key: ImageKey, size: Size) {
        let Some(entry) = self.entries.get_mut(&key) else {
            // Released while decoding and already forgotten.
            ctx.commands.push(HostCommand::ReleaseImage(key));
            return;
        };
        entry.state = EntryState::Ready(size);
        let remote = entry.remote;
        let users = entry.users.clone();
        if users.is_empty() {
            self.idle.push_back(key);
            return self.trim(ctx);
        }
        for (node, slot) in users {
            match slot {
                Slot::Main => self.shown(ctx, node, key, size, remote),
                Slot::Placeholder => self.present(ctx, node),
            }
        }
    }

    /// The main image of `id` is decoded: it replaces what was on screen.
    fn shown(&mut self, ctx: &mut Ctx<'_>, id: NodeId, key: ImageKey, size: Size, fresh_remote: bool) {
        let Some(node) = self.nodes.get_mut(&id) else { return };
        if node.main != Some(key) {
            return;
        }
        let released: Vec<ImageKey> = [node.previous.take(), node.placeholder.take()].into_iter().flatten().collect();
        let had_picture = !released.is_empty();
        node.placeholder_source.clear();
        let fade_ms = ctx.scene.node(id).map_or(DEFAULT_FADE_MS, |node| node.props().integer_or(PropKey::ImageFadeDurationMs, DEFAULT_FADE_MS)).clamp(0, 10_000);
        // Only images that arrived from the network fade in, over nothing.
        if fresh_remote && fade_ms > 0 && !ctx.reduced_motion && !had_picture {
            node.fade = 0.0;
            ctx.animator.start(Owner::Image(id), 0.0, 1.0, Curve::timing(fade_ms as u32, Easing::OutCubic), 0, ctx.now_ms);
        } else {
            node.fade = 1.0;
        }
        for old in released {
            self.release(ctx, old, id);
        }
        let uri = self.entries.get(&key).map(|entry| entry.uri.clone()).unwrap_or_default();
        let payload = WireMap::new().with("uri", uri.as_str()).with("width", f64::from(size.width)).with("height", f64::from(size.height)).encode();
        ctx.emit_if(id, PropKey::OnImageLoad, event_kind::IMAGE_LOAD, payload);
        ctx.emit_if(id, PropKey::OnImageLoadEnd, event_kind::IMAGE_LOAD_END, Vec::new());
        self.present(ctx, id);
    }

    /// The download or the decoding of `key` failed.
    pub fn failed(&mut self, ctx: &mut Ctx<'_>, key: ImageKey, message: &str) {
        let Some(entry) = self.entries.get(&key) else { return };
        let users = entry.users.clone();
        self.forget(key);
        for (id, slot) in users {
            let Some(node) = self.nodes.get_mut(&id) else { continue };
            match slot {
                Slot::Main if node.main == Some(key) => {
                    node.main = None;
                    let previous = node.previous.take();
                    if let Some(previous) = previous {
                        self.release(ctx, previous, id);
                    }
                    let payload = WireMap::new().with("error", message).encode();
                    ctx.emit_if(id, PropKey::OnImageError, event_kind::IMAGE_ERROR, payload);
                    ctx.emit_if(id, PropKey::OnImageLoadEnd, event_kind::IMAGE_LOAD_END, Vec::new());
                }
                Slot::Placeholder if node.placeholder == Some(key) => node.placeholder = None,
                _ => {}
            }
            self.present(ctx, id);
        }
    }

    /// Natural sizes of the pictures under freehand canvases that changed.
    pub fn take_natural_sizes(&mut self) -> Vec<(NodeId, Option<Size>)> {
        std::mem::take(&mut self.natural_sizes)
    }

    pub fn on_sample(&mut self, ctx: &mut Ctx<'_>, id: NodeId, value: f32) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.fade = value.clamp(0.0, 1.0);
            self.present(ctx, id);
        }
    }

    /// Pushes what the node shows now: the image, the previous one, the
    /// placeholder or nothing.
    fn present(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = self.nodes.get(&id) else { return };
        let ready = |key: Option<ImageKey>| key.filter(|key| self.entries.get(key).is_some_and(|entry| matches!(entry.state, EntryState::Ready(_))));
        let (key, opacity) = match (ready(node.main), ready(node.previous), ready(node.placeholder)) {
            (Some(key), _, _) => (Some(key), node.fade),
            (None, Some(key), _) | (None, None, Some(key)) => (Some(key), 1.0),
            (None, None, None) => (None, 1.0),
        };
        let sketch = node.sketch;
        let content = key.and_then(|key| {
            let scene_node = ctx.scene.node(id)?;
            let props = scene_node.props();
            if sketch {
                // Strokes are normalised to the aspect-fit rectangle of the whole view.
                return Some(ImageContent { key, fit: ImageFit::Contain, tint: None, overlay: None, insets: Insets::ZERO, opacity });
            }
            let [left, top, right, bottom] = scene_node.decoration.border.widths;
            Some(ImageContent { key, fit: fit(props), tint: props.color(PropKey::TintColor), overlay: None, insets: Insets::new(left, top, right, bottom), opacity })
        });
        if content != node.pushed {
            if sketch {
                let natural = key.and_then(|key| match self.entries.get(&key).map(|entry| &entry.state) {
                    Some(EntryState::Ready(size)) => Some(*size),
                    _ => None,
                });
                self.natural_sizes.push((id, natural));
            }
            ctx.commands.push(HostCommand::SetImage { id, content });
            if let Some(node) = self.nodes.get_mut(&id) {
                node.pushed = content;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_sets_pick_the_closest_candidate() {
        let set = "a.png 1x, b.png 2x, c.png 3x";
        assert_eq!(pick_from_source_set(set, 1.0, 100.0), Some("a.png"));
        assert_eq!(pick_from_source_set(set, 2.2, 100.0), Some("b.png"));
        assert_eq!(pick_from_source_set("s.png 320w, m.png 640w, l.png 1280w", 1.0, 600.0), Some("m.png"));
        assert_eq!(pick_from_source_set("plain.png, other", 1.0, 100.0), None);
        assert_eq!(pick_from_source_set("", 1.0, 100.0), None);
    }

    #[test]
    fn data_uris_decode_base64_and_percent_forms() {
        assert_eq!(data_uri("data:image/png;base64,AAEC"), Location::Bytes(Arc::from(vec![0, 1, 2])));
        assert_eq!(data_uri("data:image/svg+xml,%3Csvg%3E"), Location::Bytes(Arc::from(b"<svg>".to_vec())));
        assert!(matches!(data_uri("data:text/plain;base64,AAEC"), Location::Invalid(_)));
        assert!(matches!(data_uri("data:image/png;base64"), Location::Invalid(_)));
        assert!(matches!(data_uri("data:image/png;base64,"), Location::Invalid(_)));
    }

    #[test]
    fn asset_paths_stay_inside_the_bundle() {
        let root = Path::new("/app");
        assert_eq!(asset_path(root, "assets/logo.png"), Some(PathBuf::from("/app/assets/logo.png")));
        assert_eq!(asset_path(root, "/assets/logo.png"), Some(PathBuf::from("/app/assets/logo.png")));
        assert_eq!(asset_path(root, "../secret"), None);
        assert_eq!(asset_path(root, "a//b"), None);
        assert_eq!(asset_path(root, "a/./b"), None);
        assert_eq!(asset_path(root, "a?b"), None);
        assert_eq!(asset_path(root, ""), None);
    }

    #[test]
    fn decode_buckets_cover_the_box() {
        assert_eq!(bucket_for(48.0), 128);
        assert_eq!(bucket_for(128.0), 128);
        assert_eq!(bucket_for(129.0), 256);
        assert_eq!(bucket_for(2048.0), 2048);
        assert_eq!(bucket_for(4000.0), 0);
    }
}
