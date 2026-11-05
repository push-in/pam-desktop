//! The media player of a `Media` view: an `AVPlayer` shown by a bare
//! `AVPlayerLayer`, or by AVKit's `AVPlayerView` when the application asks
//! for the system's transport controls.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_av_foundation::{
    AVLayerVideoGravity, AVLayerVideoGravityResize, AVLayerVideoGravityResizeAspect,
    AVLayerVideoGravityResizeAspectFill, AVPlayer, AVPlayerActionAtItemEnd, AVPlayerItem,
    AVPlayerItemDidPlayToEndTimeNotification, AVPlayerItemFailedToPlayToEndTimeNotification,
    AVPlayerItemStatus, AVPlayerLayer, AVPlayerTimeControlStatus,
};
use objc2_av_kit::{AVPlayerView, AVPlayerViewControlsStyle};
use objc2_core_foundation::{CGPoint, CGRect};
use objc2_core_media::CMTime;
use objc2_foundation::{
    NSDictionary, NSKeyValueObservingOptions, NSNotification, NSNotificationCenter,
    NSObjectNSKeyValueObserverRegistration, NSObjectNSThreadPerformAdditions, NSOperationQueue,
    NSString, NSThread, NSURL,
};
use pam_desktop_ui::backend::{
    BackendEvent, ImageFit, MediaEvent, MediaSource, MediaState, NodeId,
};
use pam_desktop_ui::geometry::Size;

use crate::events::Events;
use crate::util::{cg_size, ns_string};

/// Units per second of the times handed to the player.
const TIMESCALE: i32 = 600;

/// What shows the video.
enum Surface {
    /// A layer-hosting view around a player layer (no controls).
    Layer {
        view: Retained<NSView>,
        layer: Retained<AVPlayerLayer>,
    },
    /// AVKit's player view with its inline controls.
    Controls(Retained<AVPlayerView>),
}

impl Surface {
    fn view(&self) -> Retained<NSView> {
        match self {
            Self::Layer { view, .. } => view.clone(),
            Self::Controls(view) => Retained::into_super(view.clone()),
        }
    }

    /// The object whose `readyForDisplay` tells that a frame is on screen.
    fn display(&self) -> Retained<NSObject> {
        match self {
            Self::Layer { layer, .. } => AsRef::<NSObject>::as_ref(&**layer).retain(),
            Self::Controls(view) => AsRef::<NSObject>::as_ref(&**view).retain(),
        }
    }

    fn ready_for_display(&self) -> bool {
        // SAFETY: plain property reads on the main thread.
        unsafe {
            match self {
                Self::Layer { layer, .. } => layer.isReadyForDisplay(),
                Self::Controls(view) => view.isReadyForDisplay(),
            }
        }
    }

    fn set_player(&self, player: Option<&AVPlayer>) {
        // SAFETY: plain property writes on the main thread.
        unsafe {
            match self {
                Self::Layer { layer, .. } => layer.setPlayer(player),
                Self::Controls(view) => view.setPlayer(player),
            }
        }
    }

    fn set_gravity(&self, gravity: &AVLayerVideoGravity) {
        // SAFETY: plain property writes on the main thread.
        unsafe {
            match self {
                Self::Layer { layer, .. } => layer.setVideoGravity(gravity),
                Self::Controls(view) => view.setVideoGravity(gravity),
            }
        }
    }
}

/// State shared with the observer of the player.
struct Shared {
    id: NodeId,
    events: Rc<Events>,
    player: RefCell<Option<Retained<AVPlayer>>>,
    surface: RefCell<Option<Surface>>,
    /// Position to start at once the item is ready, in seconds.
    start_at: Cell<f64>,
    /// Whether the item should play once ready, and how fast.
    play_when_ready: Cell<bool>,
    rate: Cell<f32>,
    looping: Cell<bool>,
    /// What was reported, to report each change once.
    ready: Cell<bool>,
    failed: Cell<bool>,
    rendered: Cell<bool>,
    buffering: Cell<bool>,
    playing: Cell<bool>,
    /// Playback reached the end and nothing moved it since.
    ended: Cell<bool>,
}

fn seconds(time: CMTime) -> f64 {
    // SAFETY: converts a plain value.
    let seconds = unsafe { time.seconds() };
    if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    }
}

fn time(seconds: f64) -> CMTime {
    // SAFETY: builds a plain value.
    unsafe { CMTime::with_seconds(seconds.max(0.0), TIMESCALE) }
}

impl Shared {
    fn emit(&self, event: MediaEvent) {
        self.events.push(BackendEvent::Media { id: self.id, event });
    }

    fn seek(&self, player: &AVPlayer, position: f64) {
        self.ended.set(false);
        let exact = time(0.0);
        // SAFETY: plain call on the main thread with valid times.
        unsafe { player.seekToTime_toleranceBefore_toleranceAfter(time(position), exact, exact) };
    }

    /// Compares the player with what was last reported and reports the
    /// differences. Called for every observed change.
    fn refresh(&self) {
        let Some(player) = self.player.borrow().clone() else {
            return;
        };
        // SAFETY: plain property reads on the main thread.
        let (item, rate, control) = unsafe {
            (
                player.currentItem(),
                player.rate(),
                player.timeControlStatus(),
            )
        };
        if let Some(item) = &item {
            // SAFETY: as above.
            let status = unsafe { item.status() };
            if status == AVPlayerItemStatus::ReadyToPlay && !self.ready.replace(true) {
                // SAFETY: as above.
                let (natural, duration) =
                    unsafe { (item.presentationSize(), seconds(item.duration())) };
                self.emit(MediaEvent::Ready {
                    width: natural.width.max(0.0) as u32,
                    height: natural.height.max(0.0) as u32,
                    duration,
                });
                if self.start_at.get() > 0.0 {
                    self.seek(&player, self.start_at.get());
                }
                if self.play_when_ready.get() {
                    // SAFETY: plain call on the main thread.
                    unsafe { player.setRate(self.rate.get()) };
                }
            } else if status == AVPlayerItemStatus::Failed && !self.failed.replace(true) {
                // SAFETY: plain property read on the main thread.
                let reason = unsafe { item.error() }
                    .map(|error| error.localizedDescription().to_string())
                    .unwrap_or_default();
                self.emit(MediaEvent::Failed(format!(
                    "Media playback failed ({reason})"
                )));
            }
        }
        if !self.ready.get() {
            return;
        }
        let waiting = control == AVPlayerTimeControlStatus::WaitingToPlayAtSpecifiedRate;
        if self.buffering.replace(waiting) != waiting {
            self.emit(MediaEvent::Buffering(waiting));
        }
        // While it waits for data the player still "plays": only a rate of
        // zero is a pause.
        let playing = rate != 0.0;
        if self.playing.replace(playing) != playing {
            self.emit(MediaEvent::Playing(playing));
        }
        let shows = self
            .surface
            .borrow()
            .as_ref()
            .is_some_and(Surface::ready_for_display);
        // SAFETY: plain property read on the main thread.
        let has_video = item.is_some_and(|item| unsafe { item.presentationSize() }.width > 0.0);
        if shows && has_video && !self.rendered.replace(true) {
            self.emit(MediaEvent::Rendered);
        }
    }

    fn reached_end(&self) {
        let Some(player) = self.player.borrow().clone() else {
            return;
        };
        if self.looping.get() {
            // The player keeps its rate at the end of a looping item.
            self.seek(&player, 0.0);
        } else {
            self.ended.set(true);
            self.emit(MediaEvent::Ended);
        }
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. The instance
    // variables are only touched on the main thread (see `observe_value`).
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Shared>]
    struct MediaObserver;

    impl MediaObserver {
        /// Key-value observation of the player, its item and its surface.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            if NSThread::isMainThread_class() {
                // A strong reference: an event reported from here may end
                // with the player, and this observer, destroyed.
                let shared = Rc::clone(self.ivars());
                shared.refresh();
            } else {
                // AVFoundation reports on the main thread by default; if it
                // ever does not, the change is looked at there.
                // SAFETY: `refresh:` is implemented below and takes one
                // object argument, here nil. The call only enqueues.
                unsafe { self.performSelectorOnMainThread_withObject_waitUntilDone(sel!(refresh:), None, false) };
            }
        }

        #[unsafe(method(refresh:))]
        fn refresh(&self, _argument: Option<&AnyObject>) {
            let shared = Rc::clone(self.ivars());
            shared.refresh();
        }
    }

    unsafe impl NSObjectProtocol for MediaObserver {}
);

pub struct MediaHost {
    mtm: MainThreadMarker,
    shared: Rc<Shared>,
    observer: Retained<MediaObserver>,
    /// Objects and key paths the observer is registered on.
    observed: Vec<(Retained<NSObject>, &'static str)>,
    /// Notification registrations of the current item.
    tokens: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    /// What the player was last asked to be.
    state: Option<MediaState>,
    size: Size,
}

impl MediaHost {
    pub fn new(mtm: MainThreadMarker, id: NodeId, events: &Rc<Events>) -> Self {
        let shared = Rc::new(Shared {
            id,
            events: Rc::clone(events),
            player: RefCell::new(None),
            surface: RefCell::new(None),
            start_at: Cell::new(0.0),
            play_when_ready: Cell::new(false),
            rate: Cell::new(1.0),
            looping: Cell::new(false),
            ready: Cell::new(false),
            failed: Cell::new(false),
            rendered: Cell::new(false),
            buffering: Cell::new(false),
            playing: Cell::new(false),
            ended: Cell::new(false),
        });
        let this = MediaObserver::alloc(mtm).set_ivars(Rc::clone(&shared));
        // SAFETY: `init` is the designated initializer of `NSObject`.
        let observer: Retained<MediaObserver> = unsafe { msg_send![super(this), init] };
        Self {
            mtm,
            shared,
            observer,
            observed: Vec::new(),
            tokens: Vec::new(),
            state: None,
            size: Size::ZERO,
        }
    }

    /// The view placed inside the node's view, once there is one.
    pub fn view(&self) -> Option<Retained<NSView>> {
        self.shared.surface.borrow().as_ref().map(Surface::view)
    }

    fn observe(&mut self, object: Retained<NSObject>, key: &'static str) {
        // SAFETY: the observer implements `observeValueForKeyPath:…` and is
        // removed from every object in `unobserve` before either goes away.
        unsafe {
            object.addObserver_forKeyPath_options_context(
                &self.observer,
                &ns_string(key),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        self.observed.push((object, key));
    }

    fn unobserve(&mut self, keep: impl Fn(&str) -> bool) {
        let observer = &self.observer;
        self.observed.retain(|(object, key)| {
            if keep(key) {
                return true;
            }
            // SAFETY: removes a registration made in `observe`.
            unsafe { object.removeObserver_forKeyPath(observer, &ns_string(key)) };
            false
        });
    }

    /// Shows the video with or without the system's controls.
    fn set_surface(&mut self, controls: bool, parent: &NSView) {
        let current = self
            .shared
            .surface
            .borrow()
            .as_ref()
            .map(|surface| matches!(surface, Surface::Controls(_)));
        if current == Some(controls) {
            return;
        }
        self.remove_surface();
        let player = self.shared.player.borrow().clone();
        // SAFETY: plain constructors and property writes on the main thread.
        let surface = unsafe {
            if controls {
                let view = AVPlayerView::initWithFrame(AVPlayerView::alloc(self.mtm), CGRect::ZERO);
                view.setControlsStyle(AVPlayerViewControlsStyle::Inline);
                view.setPlayer(player.as_deref());
                Surface::Controls(view)
            } else {
                let layer = AVPlayerLayer::playerLayerWithPlayer(player.as_deref());
                let view = NSView::initWithFrame(NSView::alloc(self.mtm), CGRect::ZERO);
                // Layer-hosting: the layer is set before the view is told
                // to use one.
                view.setLayer(Some(&layer));
                view.setWantsLayer(true);
                Surface::Layer { view, layer }
            }
        };
        let view = surface.view();
        view.setFrame(CGRect::new(CGPoint::ZERO, cg_size(self.size)));
        // The video sits under the node's children.
        parent.addSubview_positioned_relativeTo(
            &view,
            objc2_app_kit::NSWindowOrderingMode::Below,
            None,
        );
        self.observe(surface.display(), "readyForDisplay");
        *self.shared.surface.borrow_mut() = Some(surface);
        // A new surface shows its first frame again.
        self.shared.rendered.set(false);
    }

    fn remove_surface(&mut self) {
        self.unobserve(|key| key != "readyForDisplay");
        // The cell is free again before AppKit is called: a change it
        // reports back must be able to read it.
        let previous = self.shared.surface.borrow_mut().take();
        if let Some(previous) = previous {
            previous.set_player(None);
            previous.view().removeFromSuperview();
        }
    }

    /// Stops the player and frees its decoder.
    fn release(&mut self) {
        self.unobserve(|key| key == "readyForDisplay");
        let center = NSNotificationCenter::defaultCenter();
        for token in self.tokens.drain(..) {
            // SAFETY: removes a registration made in `open`.
            unsafe { center.removeObserver(AsRef::<AnyObject>::as_ref(&*token)) };
        }
        let player = self.shared.player.borrow_mut().take();
        if let Some(player) = player {
            // SAFETY: plain calls on the main thread.
            unsafe {
                player.pause();
                player.replaceCurrentItemWithPlayerItem(None);
            }
        }
        if let Some(surface) = self.shared.surface.borrow().as_ref() {
            surface.set_player(None);
        }
        let shared = &self.shared;
        shared.ready.set(false);
        shared.failed.set(false);
        shared.rendered.set(false);
        shared.buffering.set(false);
        shared.playing.set(false);
        shared.ended.set(false);
    }

    fn open(&mut self, source: &MediaSource) {
        let url = match source {
            MediaSource::File(path) => {
                Some(NSURL::fileURLWithPath(&ns_string(&path.to_string_lossy())))
            }
            MediaSource::Url(url) => NSURL::URLWithString(&ns_string(url)),
        };
        let Some(url) = url else {
            return self.shared.emit(MediaEvent::Failed(String::from(
                "Media playback failed (the address is not valid)",
            )));
        };
        // SAFETY: plain constructors on the main thread.
        let (item, player) = unsafe {
            let item = AVPlayerItem::playerItemWithURL(&url, self.mtm);
            let player = AVPlayer::playerWithPlayerItem(Some(&item), self.mtm);
            (item, player)
        };
        *self.shared.player.borrow_mut() = Some(player.clone());
        self.observe(AsRef::<NSObject>::as_ref(&*item).retain(), "status");
        self.observe(AsRef::<NSObject>::as_ref(&*player).retain(), "rate");
        self.observe(
            AsRef::<NSObject>::as_ref(&*player).retain(),
            "timeControlStatus",
        );

        // These notifications may be posted from any thread: they are
        // delivered on the main queue.
        let center = NSNotificationCenter::defaultCenter();
        let queue = NSOperationQueue::mainQueue();
        let shared = Rc::clone(&self.shared);
        let ended = RcBlock::new(move |_: NonNull<NSNotification>| shared.reached_end());
        let shared = Rc::clone(&self.shared);
        let failed = RcBlock::new(move |_: NonNull<NSNotification>| {
            if !shared.failed.replace(true) {
                shared.emit(MediaEvent::Failed(String::from(
                    "Media playback failed (the stream stopped)",
                )));
            }
        });
        // SAFETY: the blocks only run on the main queue, where the state
        // they capture lives; the registrations are removed in `release`.
        unsafe {
            self.tokens
                .push(center.addObserverForName_object_queue_usingBlock(
                    Some(AVPlayerItemDidPlayToEndTimeNotification),
                    Some(&item),
                    Some(&queue),
                    &ended,
                ));
            self.tokens
                .push(center.addObserverForName_object_queue_usingBlock(
                    Some(AVPlayerItemFailedToPlayToEndTimeNotification),
                    Some(&item),
                    Some(&queue),
                    &failed,
                ));
        }
        if let Some(surface) = self.shared.surface.borrow().as_ref() {
            surface.set_player(Some(&player));
        }
    }

    /// The video covers the node's box.
    pub fn layout(&mut self, size: Size) {
        self.size = size;
        if let Some(view) = self.view() {
            view.setFrame(CGRect::new(CGPoint::ZERO, cg_size(size)));
        }
    }

    pub fn apply(&mut self, state: &MediaState, parent: &NSView) {
        let previous = self.state.take();
        let source_changed =
            previous.as_ref().map(|previous| &previous.source) != Some(&state.source);
        let seek_changed = previous
            .as_ref()
            .is_some_and(|previous| previous.seek.0 != state.seek.0);
        let rate_changed = previous
            .as_ref()
            .is_some_and(|previous| previous.rate != state.rate);
        let shared = Rc::clone(&self.shared);
        shared.rate.set(state.rate.clamp(0.25, 4.0));
        shared.looping.set(state.looping);
        if source_changed {
            self.release();
            shared.start_at.set(state.seek.1.max(0.0));
            if let Some(source) = &state.source {
                self.open(source);
            }
        }
        shared.play_when_ready.set(state.playing);
        if state.source.is_some() {
            self.set_surface(state.controls, parent);
        } else {
            self.remove_surface();
        }
        if let Some(surface) = shared.surface.borrow().as_ref() {
            // SAFETY: the gravity names are immutable strings exported by
            // AVFoundation (weakly linked, hence optional).
            let gravity = unsafe {
                match state.fit {
                    ImageFit::Cover => AVLayerVideoGravityResizeAspectFill,
                    ImageFit::Fill => AVLayerVideoGravityResize,
                    ImageFit::Contain | ImageFit::Center | ImageFit::Repeat => {
                        AVLayerVideoGravityResizeAspect
                    }
                }
            };
            if let Some(gravity) = gravity {
                surface.set_gravity(gravity);
            }
        }
        let Some(player) = shared.player.borrow().clone() else {
            self.state = Some(state.clone());
            return;
        };
        // SAFETY: plain property writes and calls on the main thread.
        unsafe {
            // A looping item never pauses at its end: it is sent back to
            // the start with its rate untouched.
            player.setActionAtItemEnd(if state.looping {
                AVPlayerActionAtItemEnd::None
            } else {
                AVPlayerActionAtItemEnd::Pause
            });
            player.setMuted(state.muted);
            player.setVolume(state.volume.clamp(0.0, 1.0));
            if player.respondsToSelector(sel!(setDefaultRate:)) {
                // What the system's play button uses (macOS 13).
                player.setDefaultRate(shared.rate.get());
            }
        }
        if seek_changed && !source_changed {
            if shared.ready.get() {
                shared.seek(&player, state.seek.1);
            } else {
                shared.start_at.set(state.seek.1.max(0.0));
            }
        }
        if shared.ready.get() {
            // SAFETY: plain property read and calls on the main thread.
            unsafe {
                let playing = player.rate() != 0.0;
                if state.playing && (!playing || rate_changed) {
                    // A finished item restarts from the beginning.
                    if shared.ended.get() {
                        shared.seek(&player, 0.0);
                    }
                    player.setRate(shared.rate.get());
                } else if !state.playing && playing {
                    player.pause();
                }
            }
        }
        self.state = Some(state.clone());
    }

    /// Current position and duration of a prepared player, in seconds.
    pub fn position(&self) -> Option<(f64, f64)> {
        let player = self
            .shared
            .player
            .borrow()
            .clone()
            .filter(|_| self.shared.ready.get())?;
        // SAFETY: plain property reads on the main thread.
        unsafe {
            let duration = player
                .currentItem()
                .map_or(0.0, |item| seconds(item.duration()));
            Some((seconds(player.currentTime()), duration))
        }
    }

    pub fn destroy(&mut self) {
        self.release();
        self.remove_surface();
    }
}

impl Drop for MediaHost {
    /// The observer must not outlive its registrations (a host that was
    /// never destroyed, when the application ends).
    fn drop(&mut self) {
        self.destroy();
    }
}
