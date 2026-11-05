//! Media players: source resolution, the disk cache, the play gate
//! (intent, visibility, visible autoplay), progress reports and the events
//! of `MediaPlayer` nodes.
//!
//! The backend only owns the platform player. It receives a [`MediaState`]
//! describing what should be loaded and whether it should play, and reports
//! what the player does through [`MediaEvent`]s.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use pam_desktop_wire::{NodeKind, PropKey, PropValue};

use crate::animator::Timer;
use crate::backend::{ImageFit, MediaEvent, MediaSource, MediaState, NodeId};
use crate::context::{event_kind, Ctx, HostCommand};
use crate::modules::fetch::{HttpClient, HttpRequest};
use crate::modules::support::{copy_bounded, sha256_hex, uuid, CopyError, WorkerPool};
use crate::modules::files;
use crate::props::Props;
use crate::runtime::{Inbound, Inbox};
use crate::animator::{Curve, Easing, MotionProperty, Owner};
use crate::props::PropMap;
use crate::scene::{Change, IdMap, StyleLayer};
use crate::wiremap::WireMap;

const PROGRESS_INTERVAL_MS: u64 = 250;
/// A hidden player keeps its decoder this long before it is released.
const HIDDEN_RELEASE_MS: u64 = 5_000;
/// A player that lost its autoplay grant is released after this long.
const UNGRANTED_RELEASE_MS: u64 = 1_500;
const HYSTERESIS: f32 = 0.2;
const EPSILON: f32 = 0.001;
const DEFAULT_FILE_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
const MIN_BUDGET: u64 = 16 * 1024 * 1024;
const MAX_BUDGET: u64 = 256 * 1024 * 1024;
/// Positions remembered per source for players released by visible autoplay.
const REMEMBERED_POSITIONS: usize = 64;

pub struct MediaEnvironment {
    /// `<cache dir>/pam-media-v1`.
    pub cache_root: PathBuf,
    /// The sandbox `pam-file:///` and relative sources resolve in.
    pub files_root: PathBuf,
    /// The application bundle, where `asset://` sources resolve.
    pub assets: PathBuf,
    pub client: Option<Arc<dyn HttpClient>>,
    pub inbox: Arc<Inbox>,
}

/// Results of the cache downloads, delivered through the inbox.
#[derive(Debug)]
pub enum MediaInbound {
    Progress { node: NodeId, ticket: u64, loaded: u64, total: u64 },
    Stored { node: NodeId, ticket: u64, result: Result<(PathBuf, u64), String> },
}

/// Where a cached source lives: `<root>/<sha256(identity)>.media`.
fn cache_file(root: &Path, identity: &str) -> PathBuf {
    root.join(format!("{}.media", sha256_hex(identity.as_bytes())))
}

fn is_fresh(path: &Path, limit: u64, max_age_ms: i64) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false };
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > limit {
        return false;
    }
    if max_age_ms <= 0 {
        return true;
    }
    metadata.modified().ok().and_then(|modified| SystemTime::now().duration_since(modified).ok()).is_some_and(|age| age <= Duration::from_millis(max_age_ms as u64))
}

/// Deletes the oldest cached files beyond `budget`, sparing pinned files
/// and the newest one.
fn trim_cache(root: &Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|extension| extension == "media"))
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    let mut total = 0_u64;
    for (index, (_, size, path)) in files.iter().enumerate() {
        total = total.saturating_add(*size);
        if index > 0 && total > budget && !path.with_extension("pin").exists() {
            let _ = std::fs::remove_file(path);
            total = total.saturating_sub(*size);
        }
    }
}

/// `m:ss`, or `h:mm:ss` from one hour on.
#[must_use]
pub fn format_clock(seconds: f64) -> String {
    let total = if seconds.is_finite() { seconds.max(0.0) as u64 } else { 0 };
    let (hours, minutes, seconds) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Nothing to play, or the source failed.
    Idle,
    /// Waiting for the cache download.
    Caching,
    /// The backend has (or is preparing) a player.
    Loaded,
    /// Released while hidden or without a grant; restored on demand.
    Released,
}

struct Player {
    source: String,
    stage: Stage,
    /// What the backend plays once resolved.
    resolved: Option<MediaSource>,
    /// Identifies the current source for late download results.
    ticket: u64,
    cancel: Arc<AtomicBool>,
    /// The play intent: follows `MediaAutoPlay` when it changes, and the
    /// native controls.
    requested: bool,
    autoplay: Option<bool>,
    /// Serial and position of the last seek request.
    seek: (u32, f64),
    current_time: Option<f64>,
    ready: Option<f64>,
    /// Last position sampled, used to resume after a release.
    position: f64,
    hidden_since: Option<u64>,
    /// Visible autoplay: whether this player may play now.
    granted: bool,
    ungranted_since: Option<u64>,
    progress_armed: bool,
    pushed: Option<MediaState>,
    /// The poster covers the video until the first frame is drawn.
    rendered: bool,
    cache_signature: String,
    dirty: bool,
}

impl Player {
    fn new() -> Self {
        Self {
            source: String::new(),
            stage: Stage::Idle,
            resolved: None,
            ticket: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            requested: false,
            autoplay: None,
            seek: (0, 0.0),
            current_time: None,
            ready: None,
            position: 0.0,
            hidden_since: None,
            granted: true,
            ungranted_since: None,
            progress_armed: false,
            pushed: None,
            rendered: false,
            cache_signature: String::new(),
            dirty: true,
        }
    }
}

/// A node that shows the position of a player named by `NativeRef`: a text
/// label formatted as a clock, or a bar scaled from its leading edge.
#[derive(Clone, Debug, Default, PartialEq)]
struct Binding {
    clock: Option<(String, i64)>,
    progress: Option<String>,
    /// The fraction the bar shows, to animate small steps.
    fraction: f32,
}

/// The text of a bound clock for a position and a duration in seconds.
fn clock_text(format: i64, position: f64, duration: f64) -> String {
    let remaining = (duration - position - 1e-6).ceil().max(0.0);
    match format {
        2 => format_clock(remaining),
        3 => format!("{} / {}", format_clock(position), format_clock(duration)),
        4 => format_clock(duration),
        5 => format!("{}", remaining as u64),
        _ => format_clock(position),
    }
}

pub struct Media {
    environment: MediaEnvironment,
    players: IdMap<Player>,
    bindings: IdMap<Binding>,
    pool: WorkerPool,
    next_ticket: u64,
    /// Positions of sources released by visible autoplay, most recent last.
    remembered: Vec<(String, f64)>,
    /// Poster visibility changes for the image pipeline: node and hidden.
    posters: Vec<(NodeId, bool)>,
    dirty: bool,
    autoplay_dirty: bool,
}

#[allow(clippy::too_many_arguments)]
fn cache_event(ctx: &mut Ctx<'_>, id: NodeId, marker: PropKey, kind: u16, key: &str, loaded: u64, total: u64, disk: bool) {
    let number = |value: u64| i64::try_from(value).unwrap_or(i64::MAX);
    let payload = WireMap::new().with("key", key).with("loaded", number(loaded)).with("total", number(total)).with("disk", disk).encode();
    ctx.emit_if(id, marker, kind, payload);
}

fn fit(props: Props<'_>) -> ImageFit {
    match props.integer_or(PropKey::ImageFit, 1) {
        2 => ImageFit::Contain,
        3 => ImageFit::Fill,
        4 | 5 => ImageFit::Center,
        _ => ImageFit::Cover,
    }
}

impl Media {
    #[must_use]
    pub fn new(environment: MediaEnvironment) -> Self {
        Self {
            environment,
            players: IdMap::default(),
            bindings: IdMap::default(),
            pool: WorkerPool::new("pam-media-cache", 2),
            next_ticket: 1,
            remembered: Vec::new(),
            posters: Vec::new(),
            dirty: false,
            autoplay_dirty: false,
        }
    }

    /// Poster visibility changes since the last call: `(node, hidden)`.
    pub fn take_poster_changes(&mut self) -> Vec<(NodeId, bool)> {
        std::mem::take(&mut self.posters)
    }

    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Media) => {
                self.players.insert(id, Player::new());
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Media) => {
                if let Some(player) = self.players.remove(&id) {
                    player.cancel.store(true, Ordering::Relaxed);
                    ctx.timers.cancel(Timer::MediaProgress(id));
                    ctx.timers.cancel(Timer::MediaRelease(id));
                }
            }
            Change::Removed(id, _) => {
                self.bindings.remove(&id);
            }
            Change::Created(id, _) | Change::Updated(id, PropKey::MediaClock | PropKey::MediaClockFormat | PropKey::MediaProgress) => {
                let Some(node) = ctx.scene.node(id) else { return };
                let props = node.props();
                let clock = props.text(PropKey::MediaClock).filter(|reference| !reference.is_empty()).map(|reference| (reference.to_owned(), props.integer_or(PropKey::MediaClockFormat, 1)));
                let progress = props.text(PropKey::MediaProgress).filter(|reference| !reference.is_empty()).map(str::to_owned);
                if clock.is_none() && progress.is_none() {
                    if self.bindings.remove(&id).is_some() {
                        // The node owns its text and scale again.
                        ctx.scene.set_style_overlay(id, StyleLayer::Presenter, None);
                        let mut values = ctx.scene.motion(id);
                        values.set(MotionProperty::ScaleX, None);
                        ctx.scene.set_motion(id, values);
                    }
                    return;
                }
                self.bindings.insert(id, Binding { clock, progress, fraction: 0.0 });
            }
            Change::Updated(id, _) | Change::Frame(id) => {
                if let Some(player) = self.players.get_mut(&id) {
                    player.dirty = true;
                    self.dirty = true;
                }
            }
            _ => {}
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.players.is_empty() {
            // Routes and tabs may have covered or revealed players.
            self.autoplay_dirty = true;
        }
        if self.dirty {
            self.dirty = false;
            let dirty: Vec<NodeId> = self.players.iter().filter(|(_, player)| player.dirty).map(|(id, _)| *id).collect();
            for id in dirty {
                self.sync(ctx, id);
            }
        }
        self.evaluate(ctx);
    }

    /// Something scrolled: visibility-driven players are re-evaluated.
    pub fn on_scroll(&mut self) {
        self.autoplay_dirty = !self.players.is_empty();
    }

    /// Applies the properties of a player.
    fn sync(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let source = props.text_or_empty(PropKey::MediaSource).trim().to_owned();
        let autoplay = props.flag_or(PropKey::MediaAutoPlay, false);
        let current_time = f64::from(props.number_or(PropKey::MediaCurrentTime, 0.0).max(0.0));
        let idle_surface = props.flag_or(PropKey::MediaIdleSurface, false);
        let cache_signature = format!(
            "{}|{}|{}|{}|{}",
            props.integer_or(PropKey::MediaCachePolicy, 1),
            props.text_or_empty(PropKey::MediaCacheKey),
            props.integer_or(PropKey::MediaCacheMaxAgeMs, 0),
            props.flag_or(PropKey::MediaCacheStreaming, false) || props.flag_or(PropKey::MediaCacheDownloadWhilePlaying, false),
            props.integer_or(PropKey::MediaCacheMaxBytes, 0),
        );
        let Some(player) = self.players.get_mut(&id) else { return };
        player.dirty = false;
        // The intent follows the property only when the property changes: a
        // pause made with the native controls survives unrelated updates.
        if player.autoplay != Some(autoplay) {
            player.autoplay = Some(autoplay);
            player.requested = autoplay;
        }
        if player.current_time != Some(current_time) {
            let first = player.current_time.is_none();
            player.current_time = Some(current_time);
            if !first || current_time > 0.0 {
                player.seek = (player.seek.0.wrapping_add(1), current_time);
                player.position = current_time;
            }
        }
        let source_changed = source != player.source;
        let reload = source_changed || (cache_signature != player.cache_signature && player.stage != Stage::Idle);
        player.cache_signature = cache_signature;
        if reload {
            player.cancel.store(true, Ordering::Relaxed);
            player.cancel = Arc::new(AtomicBool::new(false));
            player.ticket = self.next_ticket;
            self.next_ticket += 1;
            if source_changed {
                player.position = current_time;
                player.seek = (player.seek.0.wrapping_add(1), current_time);
            }
            player.source.clone_from(&source);
            player.resolved = None;
            player.ready = None;
            player.stage = Stage::Idle;
            if player.rendered {
                player.rendered = false;
                self.posters.push((id, false));
            }
            if !source.is_empty() && !idle_surface {
                self.resolve(ctx, id);
            }
        } else if idle_surface && player.stage != Stage::Idle {
            // The surface stays mounted with nothing behind it.
            player.cancel.store(true, Ordering::Relaxed);
            player.stage = Stage::Idle;
            player.resolved = None;
            player.ready = None;
            player.position = current_time;
            if player.rendered {
                player.rendered = false;
                self.posters.push((id, false));
            }
        } else if !idle_surface && player.stage == Stage::Idle && !player.source.is_empty() && player.resolved.is_none() && player.pushed.as_ref().is_some_and(|state| state.source.is_none()) {
            self.resolve(ctx, id);
        }
        self.autoplay_dirty = true;
        self.push(ctx, id);
    }

    /// Turns the source string into something the backend can open, going
    /// through the disk cache when the policy asks for it.
    fn resolve(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let policy = props.integer_or(PropKey::MediaCachePolicy, 1);
        let key = props.text_or_empty(PropKey::MediaCacheKey).to_owned();
        let max_age = props.integer_or(PropKey::MediaCacheMaxAgeMs, 0);
        let streaming = props.flag_or(PropKey::MediaCacheStreaming, false) || props.flag_or(PropKey::MediaCacheDownloadWhilePlaying, false);
        let max_bytes = u64::try_from(props.integer_or(PropKey::MediaCacheMaxBytes, 0)).unwrap_or(0);
        let pin = props.flag_or(PropKey::MediaCachePinOffline, false);
        let checksum = props.text_or_empty(PropKey::MediaCacheChecksum).to_ascii_lowercase();
        let Some(player) = self.players.get_mut(&id) else { return };
        let source = player.source.clone();
        let lower = source.to_ascii_lowercase();
        let fail = |player: &mut Player, ctx: &mut Ctx<'_>, message: &str| {
            player.stage = Stage::Idle;
            player.resolved = None;
            ctx.emit_if(id, PropKey::OnMediaError, event_kind::MEDIA_ERROR, WireMap::new().with("message", message).encode());
        };
        let local = |player: &mut Player, ctx: &mut Ctx<'_>, path: Option<PathBuf>, escaped: &str| match path {
            Some(path) if path.is_file() => {
                player.resolved = Some(MediaSource::File(path));
                player.stage = Stage::Loaded;
                ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
            }
            Some(_) => fail(player, ctx, "Media does not exist."),
            None => fail(player, ctx, escaped),
        };
        if lower.starts_with("https://") || lower.starts_with("http://") {
            let identity = if key.is_empty() { source.clone() } else { key };
            let cache_key = if identity == source { sha256_hex(source.as_bytes()) } else { identity.clone() };
            let file = cache_file(&self.environment.cache_root, &identity);
            let limit = if max_bytes > 0 { max_bytes } else { DEFAULT_FILE_LIMIT };
            let budget = (if max_bytes > 0 { max_bytes } else { MAX_BUDGET }).clamp(MIN_BUDGET, MAX_BUDGET);
            if policy < 3 {
                player.resolved = Some(MediaSource::Url(source));
                player.stage = Stage::Loaded;
                ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
                return;
            }
            let fresh = is_fresh(&file, limit, max_age);
            let stored = file.is_file();
            // Cache first, except "network first", which always asks the network.
            if fresh && policy != 6 {
                // A hit refreshes the file's place in the least-recently-used order.
                let _ = std::fs::File::open(&file).and_then(|handle| handle.set_modified(SystemTime::now()));
                cache_event(ctx, id, PropKey::OnMediaCacheHit, event_kind::MEDIA_CACHE_HIT, &cache_key, 0, 0, true);
                player.resolved = Some(MediaSource::File(file));
                player.stage = Stage::Loaded;
                ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
                return;
            }
            if policy == 8 && stored {
                // Stale while revalidate: play the old file now, refresh it behind.
                cache_event(ctx, id, PropKey::OnMediaCacheHit, event_kind::MEDIA_CACHE_HIT, &cache_key, 0, 0, true);
                player.resolved = Some(MediaSource::File(file.clone()));
                player.stage = Stage::Loaded;
                ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
            }
            cache_event(ctx, id, PropKey::OnMediaCacheMiss, event_kind::MEDIA_CACHE_MISS, &cache_key, 0, 0, false);
            if policy == 7 {
                return fail(player, ctx, "Media is not available in the local cache.");
            }
            let Some(client) = self.environment.client.clone() else { return fail(player, ctx, "Media cache failed.") };
            if player.resolved.is_none() {
                if streaming {
                    player.resolved = Some(MediaSource::Url(source.clone()));
                    player.stage = Stage::Loaded;
                    ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
                } else {
                    player.stage = Stage::Caching;
                }
            }
            let (ticket, cancel, inbox, root) = (player.ticket, Arc::clone(&player.cancel), Arc::clone(&self.environment.inbox), self.environment.cache_root.clone());
            let wants_progress = ctx.scene.node(id).is_some_and(|node| node.handles(PropKey::OnMediaCacheProgress));
            self.pool.run(move || {
                let mut result = download(client.as_ref(), &root, &source, &file, limit, &cancel, |loaded, total| {
                    if wants_progress {
                        inbox.post(Inbound::Media(MediaInbound::Progress { node: id, ticket, loaded, total }));
                    }
                });
                // A file that is not the one the application expects is not kept.
                if let (Ok((path, _)), false) = (&result, checksum.is_empty())
                    && file_sha256(path).as_deref() != Some(checksum.as_str()) {
                        let _ = std::fs::remove_file(path);
                        result = Err(String::from("Media checksum verification failed."));
                    }
                if let Ok((path, _)) = &result {
                    if pin {
                        let _ = std::fs::write(path.with_extension("pin"), b"");
                    }
                    trim_cache(&root, budget);
                }
                if !cancel.load(Ordering::Relaxed) {
                    inbox.post(Inbound::Media(MediaInbound::Stored { node: id, ticket, result }));
                }
            });
        } else if lower.starts_with("pam-file://") {
            let path = files::resolve_uri(&self.environment.files_root, &source);
            local(player, ctx, path, "Media path escapes the application sandbox");
        } else if lower.starts_with("asset://") {
            let relative = &source[8..];
            let valid = !relative.is_empty() && !relative.contains(['\\', '\0', '?', '#']) && relative.split('/').all(|segment| !segment.is_empty() && segment != "." && segment != "..");
            local(player, ctx, valid.then(|| self.environment.assets.join(relative)), "Media source is invalid.");
        } else if lower.starts_with("file://") {
            let path = source[7..].strip_prefix("localhost").unwrap_or(&source[7..]);
            local(player, ctx, path.starts_with('/').then(|| PathBuf::from(path)), "Media source is invalid.");
        } else if source.contains("://") {
            fail(player, ctx, "Media source is invalid.");
        } else if Path::new(&source).is_absolute() {
            // Desktop applications hand out absolute paths (file dialogs, drops).
            local(player, ctx, Some(PathBuf::from(&source)), "Media source is invalid.");
        } else {
            let path = files::resolve(&self.environment.files_root, &source).ok();
            local(player, ctx, path, "Media path escapes the application sandbox");
        }
    }

    pub fn on_inbound(&mut self, ctx: &mut Ctx<'_>, message: MediaInbound) {
        match message {
            MediaInbound::Progress { node, ticket, loaded, total } => {
                if self.players.get(&node).is_some_and(|player| player.ticket == ticket) {
                    let key = self.cache_key(ctx, node);
                    cache_event(ctx, node, PropKey::OnMediaCacheProgress, event_kind::MEDIA_CACHE_PROGRESS, &key, loaded, total, true);
                }
            }
            MediaInbound::Stored { node, ticket, result } => {
                let key = self.cache_key(ctx, node);
                let Some(player) = self.players.get_mut(&node).filter(|player| player.ticket == ticket) else { return };
                match result {
                    Ok((path, bytes)) => {
                        cache_event(ctx, node, PropKey::OnMediaCacheReady, event_kind::MEDIA_CACHE_READY, &key, bytes, bytes, true);
                        if player.stage == Stage::Caching {
                            player.resolved = Some(MediaSource::File(path));
                            player.stage = Stage::Loaded;
                            ctx.emit_if(node, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
                        }
                    }
                    Err(message) => {
                        if player.stage == Stage::Caching {
                            player.stage = Stage::Idle;
                            ctx.emit_if(node, PropKey::OnMediaError, event_kind::MEDIA_ERROR, WireMap::new().with("message", message.as_str()).encode());
                        }
                    }
                }
                self.autoplay_dirty = true;
                self.push(ctx, node);
                self.evaluate(ctx);
            }
        }
    }

    fn cache_key(&self, ctx: &Ctx<'_>, id: NodeId) -> String {
        let authored = ctx.scene.node(id).map(|node| node.props().text_or_empty(PropKey::MediaCacheKey).to_owned()).unwrap_or_default();
        if !authored.is_empty() {
            return authored;
        }
        self.players.get(&id).map(|player| sha256_hex(player.source.as_bytes())).unwrap_or_default()
    }

    /// Re-evaluates who may play: hidden players pause (and are released
    /// after a while), visible autoplay grants one player per group.
    pub fn evaluate(&mut self, ctx: &mut Ctx<'_>) {
        if !std::mem::take(&mut self.autoplay_dirty) {
            return;
        }
        struct Candidate {
            id: NodeId,
            fraction: f32,
            threshold: f32,
            top: f32,
            granted: bool,
        }
        let mut groups: HashMap<String, Vec<Candidate>> = HashMap::new();
        let ids: Vec<NodeId> = self.players.keys().copied().collect();
        for id in &ids {
            let Some(node) = ctx.scene.node(*id) else { continue };
            let props = node.props();
            let keep_alive = props.flag_or(PropKey::MediaKeepAliveWhenCovered, false);
            let threshold = props.number_or(PropKey::MediaAutoPlayVisible, 0.0);
            let threshold = if threshold.is_finite() { threshold.clamp(0.0, 1.0) } else { 0.0 };
            let group = props.text_or_empty(PropKey::MediaAutoPlayGroup).to_owned();
            let visible = ctx.scene.is_effectively_visible(*id);
            let Some(player) = self.players.get_mut(id) else { continue };
            // Hidden: pause now, release the decoder later.
            if visible {
                player.hidden_since = None;
            } else if player.hidden_since.is_none() {
                player.hidden_since = Some(ctx.now_ms);
                if !keep_alive {
                    ctx.timers.schedule(Timer::MediaRelease(*id), ctx.now_ms + HIDDEN_RELEASE_MS);
                }
            }
            if threshold <= 0.0 {
                player.granted = true;
                player.ungranted_since = None;
                continue;
            }
            let fraction = if visible && player.stage != Stage::Idle || player.stage == Stage::Released && visible { ctx.scene.visible_fraction(*id, ctx.viewport) } else { 0.0 };
            let top = ctx.scene.window_origin(*id).y;
            let candidate = Candidate { id: *id, fraction, threshold, top, granted: player.granted };
            if group.is_empty() {
                let floor = (threshold - HYSTERESIS).max(0.01);
                let keeps = candidate.granted && fraction > 0.0 && fraction + EPSILON >= floor;
                let meets = fraction + EPSILON >= threshold;
                self.grant(ctx, *id, keeps || meets);
            } else {
                groups.entry(group).or_default().push(candidate);
            }
        }
        for (_, mut candidates) in groups {
            candidates.sort_by(|a, b| a.top.total_cmp(&b.top));
            let keeps = |candidate: &Candidate| candidate.granted && candidate.fraction > 0.0 && candidate.fraction + EPSILON >= (candidate.threshold - HYSTERESIS).max(0.01);
            let meets = |candidate: &Candidate| candidate.fraction + EPSILON >= candidate.threshold;
            // The player already playing stays while it still qualifies;
            // otherwise the topmost one that does takes over.
            let winner = candidates
                .iter()
                .find(|candidate| keeps(candidate) && meets(candidate))
                .or_else(|| candidates.iter().find(|candidate| meets(candidate)))
                .or_else(|| candidates.iter().find(|candidate| keeps(candidate)))
                .map(|candidate| candidate.id);
            // Withdrawals first, so two players of a group never overlap.
            for candidate in &candidates {
                if Some(candidate.id) != winner {
                    self.grant(ctx, candidate.id, false);
                }
            }
            if let Some(winner) = winner {
                self.grant(ctx, winner, true);
            }
        }
        for id in ids {
            self.push(ctx, id);
        }
    }

    fn grant(&mut self, ctx: &mut Ctx<'_>, id: NodeId, granted: bool) {
        let Some(player) = self.players.get_mut(&id) else { return };
        if player.granted == granted {
            return;
        }
        player.granted = granted;
        if granted {
            player.ungranted_since = None;
        } else {
            player.ungranted_since = Some(ctx.now_ms);
            ctx.timers.schedule(Timer::MediaRelease(id), ctx.now_ms + UNGRANTED_RELEASE_MS);
        }
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, timer: Timer, position: Option<(f64, f64)>) {
        match timer {
            Timer::MediaProgress(id) => {
                let Some(player) = self.players.get_mut(&id) else { return };
                player.progress_armed = false;
                let playing = player.pushed.as_ref().is_some_and(|state| state.playing && state.source.is_some());
                if !playing {
                    return;
                }
                if let Some((current, duration)) = position {
                    player.position = current;
                    let payload = WireMap::new().with("currentTime", current).with("duration", duration).encode();
                    ctx.emit_if(id, PropKey::OnMediaProgress, event_kind::MEDIA_PROGRESS, payload);
                    self.update_bindings(ctx, id, current, duration);
                }
                self.arm_progress(ctx, id);
            }
            Timer::MediaRelease(id) => {
                let keep_alive = ctx.scene.node(id).is_some_and(|node| node.props().flag_or(PropKey::MediaKeepAliveWhenCovered, false));
                let Some(player) = self.players.get_mut(&id) else { return };
                let hidden = player.hidden_since.is_some_and(|since| ctx.now_ms.saturating_sub(since) >= HIDDEN_RELEASE_MS) && !keep_alive;
                let ungranted = player.ungranted_since.is_some_and(|since| ctx.now_ms.saturating_sub(since) >= UNGRANTED_RELEASE_MS);
                if (hidden || ungranted) && player.stage == Stage::Loaded {
                    if let Some((current, duration)) = position {
                        // A position at the very end restarts from the beginning.
                        player.position = if duration > 0.0 && duration - current < 0.25 { 0.0 } else { current };
                    }
                    if ungranted {
                        self.remembered.retain(|(source, _)| *source != player.source);
                        self.remembered.push((player.source.clone(), player.position));
                        if self.remembered.len() > REMEMBERED_POSITIONS {
                            self.remembered.remove(0);
                        }
                    }
                    player.stage = Stage::Released;
                    player.ready = None;
                    if player.rendered {
                        player.rendered = false;
                        self.posters.push((id, false));
                    }
                    self.push(ctx, id);
                }
            }
            _ => {}
        }
    }

    /// Refreshes the clocks and bars bound to the player `id`.
    fn update_bindings(&mut self, ctx: &mut Ctx<'_>, id: NodeId, position: f64, duration: f64) {
        if self.bindings.is_empty() {
            return;
        }
        let Some(name) = ctx.scene.node(id).and_then(|node| node.props().text(PropKey::NativeRef)).map(str::to_owned) else { return };
        let (position, duration) = (if position.is_finite() { position.max(0.0) } else { 0.0 }, if duration.is_finite() { duration.max(0.0) } else { 0.0 });
        let fraction = if duration > 0.0 { (position / duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
        for (node, binding) in &mut self.bindings {
            if let Some((_, format)) = binding.clock.as_ref().filter(|(reference, _)| *reference == name) {
                let text = clock_text(*format, position, duration);
                if ctx.scene.plain_text(*node).as_deref() != Some(text.as_str()) {
                    let mut overlay = PropMap::new();
                    overlay.insert(PropKey::Text, PropValue::String(text));
                    ctx.scene.set_style_overlay(*node, StyleLayer::Presenter, Some(overlay));
                }
            }
            if binding.progress.as_deref() == Some(name.as_str()) {
                let rtl = ctx.scene.node(*node).is_some_and(|node| node.props().integer(PropKey::LayoutDirection) == Some(2));
                // The bar grows from its leading edge.
                let mut overlay = PropMap::new();
                overlay.insert(PropKey::TransformOriginX, PropValue::Float(if rtl { 100.0 } else { 0.0 }));
                ctx.scene.set_style_overlay(*node, StyleLayer::Interaction, Some(overlay));
                let step = fraction - binding.fraction;
                if step > 0.0 && step < 0.25 && !ctx.reduced_motion {
                    ctx.animator.start(Owner::MediaBar(*node), binding.fraction, fraction, Curve::timing(PROGRESS_INTERVAL_MS as u32, Easing::Linear), 0, ctx.now_ms);
                } else {
                    ctx.animator.cancel(Owner::MediaBar(*node), ctx.now_ms);
                    let mut values = ctx.scene.motion(*node);
                    values.set(MotionProperty::ScaleX, Some(fraction));
                    ctx.scene.set_motion(*node, values);
                }
                binding.fraction = fraction;
            }
        }
    }

    /// A frame of a bound progress bar gliding to its new fraction.
    pub fn on_bar_sample(&mut self, ctx: &mut Ctx<'_>, node: NodeId, value: f32) {
        if self.bindings.contains_key(&node) {
            let mut values = ctx.scene.motion(node);
            values.set(MotionProperty::ScaleX, Some(value.clamp(0.0, 1.0)));
            ctx.scene.set_motion(node, values);
        }
    }

    fn arm_progress(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        if let Some(player) = self.players.get_mut(&id)
            && !player.progress_armed {
                player.progress_armed = true;
                ctx.timers.schedule(Timer::MediaProgress(id), ctx.now_ms + PROGRESS_INTERVAL_MS);
            }
    }

    /// Something the platform player did.
    pub fn on_event(&mut self, ctx: &mut Ctx<'_>, id: NodeId, event: MediaEvent) {
        let looping = ctx.scene.node(id).is_some_and(|node| node.props().flag_or(PropKey::MediaLoop, false));
        let Some(player) = self.players.get_mut(&id) else { return };
        match event {
            MediaEvent::Ready { width, height, duration } => {
                player.ready = Some(duration);
                let position = player.seek.1;
                let payload = WireMap::new().with("naturalWidth", i64::from(width)).with("naturalHeight", i64::from(height)).with("duration", duration).encode();
                ctx.emit_if(id, PropKey::OnMediaReady, event_kind::MEDIA_READY, payload);
                self.update_bindings(ctx, id, position, duration);
            }
            MediaEvent::Rendered => {
                if !player.rendered {
                    player.rendered = true;
                    self.posters.push((id, true));
                }
            }
            MediaEvent::Buffering(buffering) => {
                ctx.emit_if(id, PropKey::OnMediaBuffering, event_kind::MEDIA_BUFFERING, WireMap::new().with("buffering", buffering).encode());
            }
            MediaEvent::Ended => {
                if !looping {
                    player.requested = false;
                    if let Some(duration) = player.ready {
                        player.position = 0.0;
                        let payload = WireMap::new().with("currentTime", duration).with("duration", duration).encode();
                        ctx.emit_if(id, PropKey::OnMediaProgress, event_kind::MEDIA_PROGRESS, payload);
                        self.update_bindings(ctx, id, duration, duration);
                    }
                    ctx.emit_if(id, PropKey::OnMediaEnd, event_kind::MEDIA_END, Vec::new());
                    self.push(ctx, id);
                }
            }
            MediaEvent::Failed(message) => {
                player.stage = Stage::Idle;
                player.resolved = None;
                player.ready = None;
                if player.rendered {
                    player.rendered = false;
                    self.posters.push((id, false));
                }
                let message = if message.is_empty() { String::from("Media playback failed.") } else { message };
                ctx.emit_if(id, PropKey::OnMediaError, event_kind::MEDIA_ERROR, WireMap::new().with("message", message.as_str()).encode());
                self.push(ctx, id);
            }
            // The user played or paused with the native controls.
            MediaEvent::Playing(playing) => {
                let commanded = player.pushed.as_ref().is_some_and(|state| state.playing);
                if playing != commanded {
                    player.requested = playing;
                    self.push(ctx, id);
                }
            }
        }
    }

    /// Hands the backend the state the player should be in, when it changed.
    fn push(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let (controls, looping, muted) = (props.flag_or(PropKey::MediaControls, true), props.flag_or(PropKey::MediaLoop, false), props.flag_or(PropKey::MediaMuted, false));
        let (volume, rate) = (props.number_or(PropKey::MediaVolume, 1.0).clamp(0.0, 1.0), props.number_or(PropKey::MediaPlaybackRate, 1.0).clamp(0.25, 4.0));
        let fit = fit(props);
        let visible = ctx.scene.is_effectively_visible(id);
        let remembered = self.players.get(&id).and_then(|player| self.remembered.iter().find(|(source, _)| *source == player.source).map(|(_, position)| *position));
        let Some(player) = self.players.get_mut(&id) else { return };
        // A released player comes back when it may play again.
        if player.stage == Stage::Released && visible && player.granted && player.resolved.is_some() {
            player.stage = Stage::Loaded;
            let resume = if player.position > 0.0 { player.position } else { remembered.unwrap_or(0.0) };
            player.seek = (player.seek.0.wrapping_add(1), resume);
            ctx.emit_if(id, PropKey::OnMediaLoadStart, event_kind::MEDIA_LOAD_START, Vec::new());
        }
        let loaded = player.stage == Stage::Loaded;
        let playing = loaded && player.requested && visible && player.granted;
        let state = MediaState {
            source: if loaded { player.resolved.clone() } else { None },
            playing,
            seek: player.seek,
            controls,
            looping,
            muted,
            volume,
            rate,
            fit,
        };
        if player.pushed.as_ref() != Some(&state) {
            ctx.commands.push(HostCommand::SetMedia { id, state: Box::new(state.clone()) });
            player.pushed = Some(state);
        }
        if playing {
            self.arm_progress(ctx, id);
        }
    }
}

/// Lower-case hexadecimal SHA-256 of a file, read in chunks.
fn file_sha256(path: &Path) -> Option<String> {
    use sha2::Digest as _;
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).ok()?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Some(crate::modules::support::hex(&hasher.finalize()))
}

/// Downloads `url` to `target` through a temporary file in `root`.
fn download(client: &dyn HttpClient, root: &Path, url: &str, target: &Path, limit: u64, cancel: &AtomicBool, mut progress: impl FnMut(u64, u64)) -> Result<(PathBuf, u64), String> {
    let request = HttpRequest {
        headers: vec![(String::from("Accept"), String::from("video/*,audio/*,application/octet-stream"))],
        connect_timeout: Duration::from_secs(10),
        read_timeout: Duration::from_secs(30),
        follow_redirects: true,
        ..HttpRequest::get(url)
    };
    let mut response = client.send(request, None, cancel).map_err(|_| String::from("Media cache failed."))?;
    if !(200..300).contains(&response.status) {
        return Err(format!("Media request failed with HTTP {}.", response.status));
    }
    let declared = response.content_length.unwrap_or(0);
    if declared > limit {
        return Err("Media exceeds its cache size limit.".into());
    }
    std::fs::create_dir_all(root).map_err(|_| String::from("Media cache failed."))?;
    let partial = root.join(format!("download-{}.tmp", uuid()));
    let result = (|| {
        let mut output = std::fs::File::create(&partial).map_err(|_| String::from("Media cache failed."))?;
        let mut last = 0_u64;
        let written = copy_bounded(&mut response.body, &mut output, limit, cancel, |received| {
            // One report per 256 KiB keeps the bridge quiet.
            if received - last >= 256 * 1024 {
                last = received;
                progress(received, declared);
            }
        })
        .map_err(|error| match error {
            CopyError::TooLarge => String::from("Media exceeds its cache size limit."),
            CopyError::Cancelled | CopyError::Io(_) => String::from("Media cache failed."),
        })?;
        if written == 0 || (declared > 0 && declared != written) {
            return Err(String::from("Media cache failed."));
        }
        drop(output);
        std::fs::rename(&partial, target).map_err(|_| String::from("Media cache failed."))?;
        Ok((target.to_path_buf(), written))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_use_minutes_and_hours() {
        assert_eq!(format_clock(0.0), "0:00");
        assert_eq!(format_clock(65.9), "1:05");
        assert_eq!(format_clock(3725.0), "1:02:05");
        assert_eq!(format_clock(f64::NAN), "0:00");
        assert_eq!(format_clock(-3.0), "0:00");
    }

    #[test]
    fn files_are_hashed_in_chunks() {
        let path = std::env::temp_dir().join(format!("pam-media-hash-{}", uuid()));
        std::fs::write(&path, b"abc").unwrap_or_default();
        assert_eq!(file_sha256(&path).as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
        let _ = std::fs::remove_file(&path);
        assert_eq!(file_sha256(&path), None);
    }

    #[test]
    fn cache_files_are_named_by_the_identity() {
        let path = cache_file(Path::new("/cache"), "abc");
        assert_eq!(path, PathBuf::from("/cache/ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad.media"));
    }

    #[test]
    fn trimming_spares_the_newest_and_pinned_files() {
        let root = std::env::temp_dir().join(format!("pam-media-trim-{}", uuid()));
        std::fs::create_dir_all(&root).unwrap_or_default();
        let now = SystemTime::now();
        for (name, age) in [("old", 300), ("pinned", 200), ("new", 0)] {
            let path = root.join(format!("{name}.media"));
            std::fs::write(&path, vec![0_u8; 1024]).unwrap_or_default();
            if let Ok(file) = std::fs::File::options().write(true).open(&path) {
                let _ = file.set_modified(now - Duration::from_secs(age));
            }
        }
        std::fs::write(root.join("pinned.pin"), b"").unwrap_or_default();
        trim_cache(&root, 1024);
        assert!(root.join("new.media").exists());
        assert!(root.join("pinned.media").exists());
        assert!(!root.join("old.media").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}
