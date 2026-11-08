//! The media player of a `Media` view: a `GtkMediaFile` (GStreamer) shown
//! by a picture, with GTK's transport controls over it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use pam_desktop_ui::backend::{
    BackendEvent, ImageFit, MediaEvent, MediaSource, MediaState, NodeId,
};
use pam_desktop_ui::runtime::EventQueue;

use crate::view::PamView;

const MICROSECONDS: f64 = 1_000_000.0;

pub struct MediaHost {
    id: NodeId,
    events: Rc<EventQueue>,
    overlay: gtk::Overlay,
    picture: gtk::Picture,
    controls: gtk::MediaControls,
    revealer: gtk::Revealer,
    stream: Option<gtk::MediaFile>,
    /// What the player was last asked to be.
    state: Option<MediaState>,
    /// Position to start at once the stream is prepared, in seconds.
    start_at: Rc<Cell<f64>>,
    /// Whether the stream should play once prepared.
    play_when_ready: Rc<Cell<bool>>,
    rendered: Rc<Cell<bool>>,
    hovered: Rc<Cell<bool>>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
}

impl MediaHost {
    pub fn new(id: NodeId, container: &PamView, events: &Rc<EventQueue>) -> Self {
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        crate::compat::set_content_fit(&picture, ImageFit::Cover);
        let controls = gtk::MediaControls::new(gtk::MediaStream::NONE);
        let revealer = gtk::Revealer::builder()
            .transition_type(gtk::RevealerTransitionType::Crossfade)
            .valign(gtk::Align::End)
            .child(&controls)
            .build();
        controls.add_css_class("osd");
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&picture));
        overlay.add_overlay(&revealer);
        overlay.set_overflow(gtk::Overflow::Hidden);
        overlay.set_parent(container);

        // The controls show while the pointer is over the player.
        let hovered = Rc::new(Cell::new(false));
        let motion = gtk::EventControllerMotion::new();
        {
            let (revealer, hovered) = (revealer.downgrade(), Rc::clone(&hovered));
            motion.connect_enter(move |_, _, _| {
                hovered.set(true);
                if let Some(revealer) = revealer.upgrade() {
                    revealer.set_reveal_child(revealer.is_visible());
                }
            });
        }
        {
            let (revealer, hovered) = (revealer.downgrade(), Rc::clone(&hovered));
            motion.connect_leave(move |_| {
                hovered.set(false);
                if let Some(revealer) = revealer.upgrade() {
                    revealer.set_reveal_child(false);
                }
            });
        }
        overlay.add_controller(motion);

        Self {
            id,
            events: Rc::clone(events),
            overlay,
            picture,
            controls,
            revealer,
            stream: None,
            state: None,
            start_at: Rc::new(Cell::new(0.0)),
            play_when_ready: Rc::new(Cell::new(false)),
            rendered: Rc::new(Cell::new(false)),
            hovered,
            handlers: RefCell::new(Vec::new()),
        }
    }

    pub fn destroy(&mut self) {
        self.release();
        self.overlay.unparent();
    }

    /// Stops the player and frees its decoder.
    fn release(&mut self) {
        if let Some(stream) = self.stream.take() {
            for handler in self.handlers.borrow_mut().drain(..) {
                stream.disconnect(handler);
            }
            stream.set_playing(false);
            self.controls.set_media_stream(gtk::MediaStream::NONE);
            self.picture.set_paintable(gdk::Paintable::NONE);
            stream.clear();
        }
        self.rendered.set(false);
    }

    fn open(&mut self, source: &MediaSource) {
        let stream = match source {
            MediaSource::File(path) => gtk::MediaFile::for_filename(path),
            MediaSource::Url(url) => gtk::MediaFile::for_file(&gio::File::for_uri(url)),
        };
        let id = self.id;
        let mut handlers = Vec::new();
        {
            let (events, start_at, play_when_ready) = (
                Rc::clone(&self.events),
                Rc::clone(&self.start_at),
                Rc::clone(&self.play_when_ready),
            );
            handlers.push(stream.connect_prepared_notify(move |stream| {
                if !stream.is_prepared() {
                    return;
                }
                let duration = stream.duration() as f64 / MICROSECONDS;
                let (width, height) = (
                    stream.intrinsic_width().max(0) as u32,
                    stream.intrinsic_height().max(0) as u32,
                );
                events.push(BackendEvent::Media {
                    id,
                    event: MediaEvent::Ready {
                        width,
                        height,
                        duration: duration.max(0.0),
                    },
                });
                let start = start_at.get();
                if start > 0.0 && stream.is_seekable() {
                    stream.seek((start * MICROSECONDS) as i64);
                }
                if play_when_ready.get() {
                    stream.play();
                }
            }));
        }
        {
            let events = Rc::clone(&self.events);
            handlers.push(stream.connect_ended_notify(move |stream| {
                if stream.is_ended() && !stream.is_loop() {
                    events.push(BackendEvent::Media {
                        id,
                        event: MediaEvent::Ended,
                    });
                }
            }));
        }
        {
            let events = Rc::clone(&self.events);
            handlers.push(stream.connect_error_notify(move |stream| {
                if let Some(error) = stream.error() {
                    events.push(BackendEvent::Media {
                        id,
                        event: MediaEvent::Failed(format!(
                            "Media playback failed ({})",
                            error.message()
                        )),
                    });
                }
            }));
        }
        {
            let events = Rc::clone(&self.events);
            handlers.push(stream.connect_playing_notify(move |stream| {
                events.push(BackendEvent::Media {
                    id,
                    event: MediaEvent::Playing(stream.is_playing()),
                });
            }));
        }
        {
            let (events, rendered) = (Rc::clone(&self.events), Rc::clone(&self.rendered));
            handlers.push(stream.connect_invalidate_contents(move |stream| {
                // The first new contents after preparation is the first frame.
                if !rendered.get() && stream.is_prepared() && stream.has_video() {
                    rendered.set(true);
                    events.push(BackendEvent::Media {
                        id,
                        event: MediaEvent::Rendered,
                    });
                }
            }));
        }
        *self.handlers.borrow_mut() = handlers;
        self.picture.set_paintable(Some(&stream));
        self.controls.set_media_stream(Some(&stream));
        self.stream = Some(stream);
    }

    pub fn apply(&mut self, state: &MediaState) {
        let previous = self.state.take();
        let source_changed =
            previous.as_ref().map(|previous| &previous.source) != Some(&state.source);
        let seek_changed = previous
            .as_ref()
            .is_some_and(|previous| previous.seek.0 != state.seek.0);
        if source_changed {
            self.release();
            self.start_at.set(state.seek.1.max(0.0));
            self.play_when_ready.set(state.playing);
            if let Some(source) = &state.source {
                self.open(source);
            }
        }
        self.play_when_ready.set(state.playing);
        crate::compat::set_content_fit(&self.picture, state.fit);
        self.revealer.set_visible(state.controls);
        self.revealer
            .set_reveal_child(state.controls && self.hovered.get());
        if let Some(stream) = &self.stream {
            stream.set_loop(state.looping);
            stream.set_muted(state.muted);
            stream.set_volume(f64::from(state.volume));
            if seek_changed && !source_changed {
                if stream.is_prepared() {
                    if stream.is_seekable() {
                        stream.seek((state.seek.1.max(0.0) * MICROSECONDS) as i64);
                    }
                } else {
                    self.start_at.set(state.seek.1.max(0.0));
                }
            }
            if stream.is_prepared() && stream.is_playing() != state.playing {
                // A finished stream restarts from the beginning.
                if state.playing && stream.is_ended() && stream.is_seekable() {
                    stream.seek(0);
                }
                stream.set_playing(state.playing);
            }
        }
        self.state = Some(state.clone());
    }

    pub fn position(&self) -> Option<(f64, f64)> {
        let stream = self.stream.as_ref().filter(|stream| stream.is_prepared())?;
        Some((
            stream.timestamp() as f64 / MICROSECONDS,
            (stream.duration() as f64 / MICROSECONDS).max(0.0),
        ))
    }
}
