//! The media player of a `Media` view: a Media Foundation media engine in
//! frame-server mode.
//!
//! The engine decodes on its own threads and hands each video frame to a
//! Direct3D texture on request; the texture is drawn inside the Direct2D
//! scene like any image. That keeps video under the same clipping, rounded
//! corners, opacity and overlays as the rest of the tree, which a child
//! window (windowed mode) could not do. Frames are only pulled on vertical
//! blanks while a player has something new to show.

use std::rc::Rc;
use std::sync::Arc;

use pam_desktop_ui::backend::{BackendEvent, ImageFit, MediaEvent, MediaSource, MediaState, NodeId, WindowId};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Point, Rect, Size};
use pam_desktop_ui::text::{FontSpec, TextBlock, TextRun};
use windows::core::{implement, Interface, BSTR};
use windows::Win32::Foundation::{RECT, S_OK};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_IGNORE, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1Brush, ID2D1DeviceContext, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_INTERPOLATION_MODE_LINEAR, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISurface;
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFDXGIDeviceManager, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineNotify,
    IMFMediaEngineNotify_Impl, MFCreateAttributes, MFCreateDXGIDeviceManager, MFShutdown, MFStartup, MFSTARTUP_FULL, MF_MEDIA_ENGINE_CALLBACK,
    MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_ERR_ABORTED, MF_MEDIA_ENGINE_ERR_DECODE, MF_MEDIA_ENGINE_ERR_ENCRYPTED, MF_MEDIA_ENGINE_ERR_NETWORK,
    MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED, MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED, MF_MEDIA_ENGINE_EVENT_BUFFERINGSTARTED, MF_MEDIA_ENGINE_EVENT_CANPLAY,
    MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR, MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY, MF_MEDIA_ENGINE_EVENT_LOADEDDATA,
    MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA, MF_MEDIA_ENGINE_EVENT_PAUSE, MF_MEDIA_ENGINE_EVENT_PLAYING, MF_MEDIA_ENGINE_EVENT_SEEKED,
    MF_MEDIA_ENGINE_EVENT_WAITING, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_VERSION,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::app::{App, Mail, Mailbox, State};
use crate::gfx::{Device, Graphics};
use crate::text::TextEngine;
use crate::util::{point_vec, rect_f, vec2};

/// Largest side of the texture video frames are copied to.
const MAX_FRAME_SIDE: u32 = 4096;
/// Ticks spent waiting for a frame after a load or a seek.
const FRAME_WAIT_TICKS: u8 = 90;
const CONTROLS_HEIGHT: f32 = 36.0;
const CONTROLS_MARGIN: f32 = 8.0;

/// Posts engine events (raised on Media Foundation threads) to the UI thread.
#[implement(IMFMediaEngineNotify)]
struct EngineNotify {
    player: u64,
    mailbox: Arc<Mailbox>,
}

/// Engine events the player acts on. Everything else (time updates four
/// times a second, download progress…) never wakes the UI thread.
const HANDLED_EVENTS: [i32; 12] = [
    MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0,
    MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0,
    MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0,
    MF_MEDIA_ENGINE_EVENT_SEEKED.0,
    MF_MEDIA_ENGINE_EVENT_PLAYING.0,
    MF_MEDIA_ENGINE_EVENT_PAUSE.0,
    MF_MEDIA_ENGINE_EVENT_WAITING.0,
    MF_MEDIA_ENGINE_EVENT_BUFFERINGSTARTED.0,
    MF_MEDIA_ENGINE_EVENT_CANPLAY.0,
    MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED.0,
    MF_MEDIA_ENGINE_EVENT_ENDED.0,
    MF_MEDIA_ENGINE_EVENT_ERROR.0,
];

impl IMFMediaEngineNotify_Impl for EngineNotify_Impl {
    fn EventNotify(&self, event: u32, first: usize, second: u32) -> windows::core::Result<()> {
        if HANDLED_EVENTS.contains(&(event as i32)) {
            self.mailbox.post(Mail::Media { player: self.player, event, first, second });
        }
        Ok(())
    }
}

/// Media Foundation objects shared by every player.
#[derive(Default)]
pub struct MediaShared {
    started: bool,
    factory: Option<IMFMediaEngineClassFactory>,
    manager: Option<IMFDXGIDeviceManager>,
    /// Device generation the manager was bound to.
    manager_generation: u64,
    next_token: u64,
    /// Engines of a lost device, waiting to be shut down outside the state borrow.
    retired: Vec<IMFMediaEngine>,
}

impl MediaShared {
    /// Starts Media Foundation and binds its device manager to the current device.
    fn prepare(&mut self, graphics: &mut Graphics) -> Result<(IMFMediaEngineClassFactory, Option<IMFDXGIDeviceManager>), String> {
        let failed = |error: windows::core::Error| format!("Media Foundation is not available ({})", error.message());
        // SAFETY: Media Foundation start-up and COM activation calls without
        // preconditions; out pointers refer to live locals.
        unsafe {
            if !self.started {
                MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(failed)?;
                self.started = true;
            }
            if self.factory.is_none() {
                self.factory = Some(CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER).map_err(failed)?);
            }
            graphics.ensure_device();
            if self.manager.is_none() || self.manager_generation != graphics.generation {
                self.manager = None;
                if let Some(device) = graphics.device.as_ref().filter(|device| device.video) {
                    let mut token = 0_u32;
                    let mut manager = None;
                    if MFCreateDXGIDeviceManager(&mut token, &mut manager).is_ok()
                        && let Some(manager) = manager
                            && manager.ResetDevice(&device.d3d, token).is_ok() {
                                self.manager = Some(manager);
                                self.manager_generation = graphics.generation;
                            }
                }
            }
        }
        let factory = self.factory.clone().ok_or_else(|| String::from("Media Foundation is not available"))?;
        Ok((factory, self.manager.clone()))
    }
}

pub struct MediaPlayer {
    /// Identifies the current engine in events (0 = none).
    token: u64,
    engine: Option<IMFMediaEngine>,
    /// What the player was last asked to be.
    state: Option<MediaState>,
    /// Position to start at once the media is prepared, in seconds.
    start_at: f64,
    /// Whether to play once prepared.
    play_when_ready: bool,
    ready: bool,
    rendered: bool,
    buffering: bool,
    /// Video size in pixels (0 for audio).
    natural: (u32, u32),
    duration: f64,
    texture: Option<ID3D11Texture2D>,
    frame: Option<ID2D1Bitmap1>,
    /// A frame was copied into the texture.
    has_frame: bool,
    /// Ticks left to wait for a frame while paused (after a load or seek).
    awaiting_frame: u8,
    fit: ImageFit,
    controls: bool,
    hovered: bool,
    /// Position to resume from after a device loss.
    resume: Option<(f64, bool)>,
}

impl MediaPlayer {
    pub fn new() -> Self {
        Self {
            token: 0,
            engine: None,
            state: None,
            start_at: 0.0,
            play_when_ready: false,
            ready: false,
            rendered: false,
            buffering: false,
            natural: (0, 0),
            duration: 0.0,
            texture: None,
            frame: None,
            has_frame: false,
            awaiting_frame: 0,
            fit: ImageFit::Cover,
            controls: false,
            hovered: false,
            resume: None,
        }
    }

    /// Detaches the engine and forgets everything about its media. The
    /// caller shuts the engine down.
    fn detach(&mut self) -> Option<IMFMediaEngine> {
        self.token = 0;
        self.ready = false;
        self.rendered = false;
        self.buffering = false;
        self.natural = (0, 0);
        self.duration = 0.0;
        self.texture = None;
        self.frame = None;
        self.has_frame = false;
        self.awaiting_frame = 0;
        self.engine.take()
    }

    fn open(&mut self, shared: &mut MediaShared, graphics: &mut Graphics, mailbox: &Arc<Mailbox>, source: &MediaSource) -> Result<(), String> {
        let (factory, manager) = shared.prepare(graphics)?;
        shared.next_token += 1;
        let token = shared.next_token;
        let failed = |error: windows::core::Error| format!("Media playback failed ({})", error.message());
        let notify: IMFMediaEngineNotify = EngineNotify { player: token, mailbox: Arc::clone(mailbox) }.into();
        let url = match source {
            MediaSource::Url(url) => url.clone(),
            MediaSource::File(path) => path.display().to_string(),
        };
        // SAFETY: COM calls on live objects; the attribute store holds its
        // own references to what is put in it.
        let engine = unsafe {
            let mut attributes: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut attributes, 3).map_err(failed)?;
            let attributes = attributes.ok_or_else(|| String::from("Media playback failed"))?;
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(failed)?;
            attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32).map_err(failed)?;
            if let Some(manager) = &manager {
                attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, manager).map_err(failed)?;
            }
            // No flags: frame-server mode with audio and video.
            let engine = factory.CreateInstance(0, &attributes).map_err(failed)?;
            let _ = engine.SetAutoPlay(false);
            if let Err(error) = engine.SetSource(&BSTR::from(url)).and_then(|()| engine.Load()) {
                // An engine is only released by its shutdown.
                let _ = engine.Shutdown();
                return Err(failed(error));
            }
            engine
        };
        self.token = token;
        self.engine = Some(engine);
        Ok(())
    }

    /// Applies what changes live. `previous` is the state the same engine
    /// was last brought to (`None` for a new engine: everything is set).
    fn apply_live(&self, state: &MediaState, previous: Option<&MediaState>) {
        let Some(engine) = &self.engine else { return };
        let rate = f64::from(state.rate.clamp(0.25, 4.0));
        // SAFETY: plain setters on the live engine.
        unsafe {
            if previous.is_none_or(|previous| previous.looping != state.looping) {
                let _ = engine.SetLoop(state.looping);
            }
            if previous.is_none_or(|previous| previous.muted != state.muted) {
                let _ = engine.SetMuted(state.muted);
            }
            if previous.is_none_or(|previous| previous.volume != state.volume) {
                let _ = engine.SetVolume(f64::from(state.volume.clamp(0.0, 1.0)));
            }
            // Setting the rate again mid-playback can glitch the audio.
            if previous.is_none_or(|previous| previous.rate != state.rate) {
                // The default rate is the one `Play` starts with.
                let _ = engine.SetDefaultPlaybackRate(rate);
                let _ = engine.SetPlaybackRate(rate);
            }
        }
    }

    fn is_playing(&self) -> bool {
        // SAFETY: plain getters on the live engine.
        self.engine.as_ref().is_some_and(|engine| unsafe { !engine.IsPaused().as_bool() && !engine.IsEnded().as_bool() })
    }

    fn position(&self) -> Option<(f64, f64)> {
        let engine = self.engine.as_ref().filter(|_| self.ready)?;
        // SAFETY: plain getter on the live engine.
        let time = unsafe { engine.GetCurrentTime() };
        Some((if time.is_finite() { time.max(0.0) } else { 0.0 }, self.duration))
    }

    /// The texture video frames are copied to, and its Direct2D bitmap.
    fn create_frame(&mut self, device: &Device) {
        let (width, height) = (self.natural.0.min(MAX_FRAME_SIDE), self.natural.1.min(MAX_FRAME_SIDE));
        if width == 0 || height == 0 {
            return;
        }
        let description = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let properties = D2D1_BITMAP_PROPERTIES1 {
            // Video frames are opaque.
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: Default::default(),
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        // SAFETY: COM calls on the live device; the descriptions outlive the calls.
        unsafe {
            let mut texture = None;
            if device.d3d.CreateTexture2D(&description, None, Some(&mut texture)).is_err() {
                return;
            }
            let Some(texture) = texture else { return };
            let Ok(surface) = texture.cast::<IDXGISurface>() else { return };
            if let Ok(bitmap) = device.context.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) {
                self.texture = Some(texture);
                self.frame = Some(bitmap);
            }
        }
    }

    /// Copies the frame due now, if the engine has a new one. Returns
    /// whether the picture changed.
    fn pull_frame(&mut self) -> bool {
        let (Some(engine), Some(texture)) = (&self.engine, &self.texture) else { return false };
        let (width, height) = (self.natural.0.min(MAX_FRAME_SIDE), self.natural.1.min(MAX_FRAME_SIDE));
        let mut presentation_time = 0_i64;
        // SAFETY: the raw vtable call is the generated wrapper's own call;
        // it is made directly because S_FALSE ("no new frame") is a success
        // code the wrapper cannot tell from S_OK. The texture belongs to the
        // device the engine decodes with.
        unsafe {
            let ready = (Interface::vtable(engine).OnVideoStreamTick)(Interface::as_raw(engine), &mut presentation_time);
            if ready != S_OK {
                return false;
            }
            let target = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
            engine.TransferVideoFrame(texture, None, &target, None).is_ok()
        }
    }

    fn controls_visible(&self) -> bool {
        self.controls && self.hovered && self.ready
    }

    /// The video frame, fitted in the box (the view's clip cuts the
    /// overflow). Returns whether the player wants the next vertical blank:
    /// a player is only asked for frames while it is painted.
    pub fn paint_video(&self, context: &ID2D1DeviceContext, _device: &Device, size: Size) -> bool {
        let wants_frames = self.texture.is_some() && (self.is_playing() || self.awaiting_frame > 0);
        let Some(frame) = self.frame.as_ref().filter(|_| self.has_frame) else { return wants_frames };
        let natural = Size::new(self.natural.0 as f32, self.natural.1 as f32);
        if natural.is_empty() || size.is_empty() {
            return wants_frames;
        }
        let scale = match self.fit {
            ImageFit::Cover => (size.width / natural.width).max(size.height / natural.height),
            ImageFit::Contain => (size.width / natural.width).min(size.height / natural.height),
            // Natural size, but never larger than the box.
            ImageFit::Center | ImageFit::Repeat => (size.width / natural.width).min(size.height / natural.height).min(1.0),
            ImageFit::Fill => 1.0,
        };
        let target = if self.fit == ImageFit::Fill {
            Rect::from_size(size)
        } else {
            let (width, height) = (natural.width * scale, natural.height * scale);
            Rect::new((size.width - width) / 2.0, (size.height - height) / 2.0, width, height)
        };
        // SAFETY: COM calls with live objects inside BeginDraw; the clip is popped.
        unsafe {
            context.PushAxisAlignedClip(&rect_f(Rect::from_size(size)), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            context.DrawBitmap(frame, Some(&rect_f(target)), 1.0, D2D1_INTERPOLATION_MODE_LINEAR, None, None);
            context.PopAxisAlignedClip();
        }
        wants_frames
    }

    /// Transport controls over the player while the pointer is on it.
    pub fn paint_controls(&self, context: &ID2D1DeviceContext, device: &Device, graphics: &Graphics, text: &TextEngine, size: Size) {
        if !self.controls_visible() {
            return;
        }
        let Some(layout) = ControlsLayout::new(size) else { return };
        let Some(engine) = &self.engine else { return };
        // SAFETY: plain getters on the live engine.
        let (paused, muted, time) = unsafe { (engine.IsPaused().as_bool() || engine.IsEnded().as_bool(), engine.GetMuted().as_bool(), engine.GetCurrentTime()) };
        let white = Color::rgba(255, 255, 255, 235);
        // SAFETY: COM calls with live objects inside BeginDraw; sinks are closed before use.
        unsafe {
            context.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect_f(layout.bar), radiusX: 8.0, radiusY: 8.0 }, device.solid(Color::rgba(0, 0, 0, 150)));
            let center = layout.play.center();
            if paused {
                // A triangle pointing right.
                if let Ok(path) = graphics.factory.CreatePathGeometry()
                    && let Ok(sink) = path.Open() {
                        sink.BeginFigure(vec2(center.x - 5.0, center.y - 7.0), D2D1_FIGURE_BEGIN_FILLED);
                        sink.AddLine(vec2(center.x + 7.0, center.y));
                        sink.AddLine(vec2(center.x - 5.0, center.y + 7.0));
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                        if sink.Close().is_ok() {
                            context.FillGeometry(&path, device.solid(white), None::<&ID2D1Brush>);
                        }
                    }
            } else {
                context.FillRectangle(&rect_f(Rect::new(center.x - 6.0, center.y - 7.0, 4.0, 14.0)), device.solid(white));
                context.FillRectangle(&rect_f(Rect::new(center.x + 2.0, center.y - 7.0, 4.0, 14.0)), device.solid(white));
            }
            // Seek track and progress.
            let track = layout.track;
            let radius = track.height / 2.0;
            context.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect_f(track), radiusX: radius, radiusY: radius }, device.solid(Color::rgba(255, 255, 255, 70)));
            let progress = if self.duration > 0.0 && time.is_finite() { (time / self.duration).clamp(0.0, 1.0) as f32 } else { 0.0 };
            let played = Rect::new(track.x, track.y, track.width * progress, track.height);
            if played.width > 0.5 {
                context.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect_f(played), radiusX: radius, radiusY: radius }, device.solid(white));
            }
            // Elapsed and total time.
            let label = TextBlock {
                runs: vec![TextRun {
                    text: format!("{} / {}", clock(time), clock(self.duration)),
                    font: FontSpec { size: 12.0, ..FontSpec::default() },
                    color: None,
                    background: None,
                    underline: false,
                    strikethrough: false,
                    letter_spacing: 0.0,
                    press: None,
                }],
                max_lines: 1,
                ..TextBlock::default()
            };
            if let Some(built) = text.build(&graphics.dwrite, &label, layout.time.width) {
                let top = layout.time.y + (layout.time.height - built.height) / 2.0;
                context.DrawTextLayout(vec2(layout.time.x, top), &built.layout, device.solid(white), D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            // Speaker: a box and a cone, crossed out when muted.
            let speaker = layout.mute.center();
            context.FillRectangle(&rect_f(Rect::new(speaker.x - 7.0, speaker.y - 3.0, 4.0, 6.0)), device.solid(white));
            if let Ok(path) = graphics.factory.CreatePathGeometry()
                && let Ok(sink) = path.Open() {
                    sink.BeginFigure(vec2(speaker.x - 3.0, speaker.y - 3.0), D2D1_FIGURE_BEGIN_FILLED);
                    sink.AddLine(vec2(speaker.x + 3.0, speaker.y - 7.0));
                    sink.AddLine(vec2(speaker.x + 3.0, speaker.y + 7.0));
                    sink.AddLine(vec2(speaker.x - 3.0, speaker.y + 3.0));
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        context.FillGeometry(&path, device.solid(white), None::<&ID2D1Brush>);
                    }
                }
            if muted {
                let (from, to) = (Point::new(speaker.x - 8.0, speaker.y - 8.0), Point::new(speaker.x + 8.0, speaker.y + 8.0));
                context.DrawLine(point_vec(from), point_vec(to), device.solid(Color::rgba(255, 90, 90, 255)), 2.0, &graphics.round_stroke);
            }
        }
    }
}

/// `m:ss` (or `h:mm:ss`) of a time in seconds.
fn clock(seconds: f64) -> String {
    let total = if seconds.is_finite() { seconds.max(0.0) as u64 } else { 0 };
    let (hours, minutes, seconds) = (total / 3600, (total / 60) % 60, total % 60);
    if hours > 0 { format!("{hours}:{minutes:02}:{seconds:02}") } else { format!("{minutes}:{seconds:02}") }
}

/// Where the transport controls go in a player of a given size.
struct ControlsLayout {
    bar: Rect,
    play: Rect,
    time: Rect,
    track: Rect,
    mute: Rect,
}

impl ControlsLayout {
    fn new(size: Size) -> Option<Self> {
        if size.width < 200.0 || size.height < CONTROLS_HEIGHT + 2.0 * CONTROLS_MARGIN {
            return None;
        }
        let bar = Rect::new(CONTROLS_MARGIN, size.height - CONTROLS_HEIGHT - CONTROLS_MARGIN, size.width - 2.0 * CONTROLS_MARGIN, CONTROLS_HEIGHT);
        let play = Rect::new(bar.x, bar.y, CONTROLS_HEIGHT, CONTROLS_HEIGHT);
        let mute = Rect::new(bar.right() - CONTROLS_HEIGHT, bar.y, CONTROLS_HEIGHT, CONTROLS_HEIGHT);
        let time = Rect::new(play.right(), bar.y, 96.0, CONTROLS_HEIGHT);
        let track_start = time.right() + 8.0;
        let track = Rect::new(track_start, bar.y + (CONTROLS_HEIGHT - 4.0) / 2.0, (mute.x - 8.0 - track_start).max(0.0), 4.0);
        Some(Self { bar, play, time, track, mute })
    }
}

fn player_mut(state: &mut State, id: NodeId) -> Option<&mut MediaPlayer> {
    state.scene.views.get_mut(&id)?.media.as_deref_mut()
}

fn shut_down(engine: IMFMediaEngine) {
    // SAFETY: the engine is live; shutting down releases its decoder and
    // threads. Called with no state borrow held.
    unsafe {
        let _ = engine.Shutdown();
    }
}

/// `Backend::set_media`: brings the player of `id` to `wanted`.
pub fn apply(app: &Rc<App>, id: NodeId, wanted: &MediaState) {
    let mailbox = Arc::clone(&app.mailbox);
    let outcome = app.with_state(|state| {
        let State { scene, graphics, media: shared, .. } = state;
        let player = scene.views.get_mut(&id)?.media.as_deref_mut()?;
        let previous = player.state.take();
        let source_changed = previous.as_ref().map(|previous| &previous.source) != Some(&wanted.source);
        let seek_changed = previous.as_ref().is_some_and(|previous| previous.seek.0 != wanted.seek.0);
        let mut retired = None;
        let mut failure = None;
        if source_changed {
            retired = player.detach();
            // Whatever was to be resumed after a device loss belonged to the old source.
            player.resume = None;
            player.start_at = wanted.seek.1.max(0.0);
            if let Some(source) = &wanted.source
                && let Err(message) = player.open(shared, graphics, &mailbox, source) {
                    failure = Some(message);
                }
        }
        player.play_when_ready = wanted.playing;
        player.fit = wanted.fit;
        player.controls = wanted.controls;
        player.apply_live(wanted, previous.as_ref().filter(|_| !source_changed));
        // An engine reopening after a device loss resumes with what is wanted now.
        if let Some(resume) = &mut player.resume {
            resume.1 = wanted.playing;
            if seek_changed {
                resume.0 = wanted.seek.1.max(0.0);
            }
        }
        if let Some(engine) = &player.engine {
            // SAFETY: plain calls on the live engine.
            unsafe {
                if seek_changed && !source_changed {
                    if player.ready {
                        let _ = engine.SetCurrentTime(wanted.seek.1.max(0.0));
                        player.awaiting_frame = FRAME_WAIT_TICKS;
                    } else {
                        player.start_at = wanted.seek.1.max(0.0);
                    }
                }
                if player.ready && player.is_playing() != wanted.playing {
                    if wanted.playing {
                        // A finished media restarts from the beginning.
                        if engine.IsEnded().as_bool() {
                            let _ = engine.SetCurrentTime(0.0);
                        }
                        let _ = engine.Play();
                    } else {
                        let _ = engine.Pause();
                    }
                }
            }
        }
        player.state = Some(wanted.clone());
        state.scene.damage_view(id);
        state.request_tick();
        Some((retired, failure))
    });
    let Some(Some((retired, failure))) = outcome else { return };
    if let Some(engine) = retired {
        shut_down(engine);
    }
    if let Some(message) = failure {
        app.emit(BackendEvent::Media { id, event: MediaEvent::Failed(message) });
    }
}

/// `Backend::media_position`.
pub fn position(app: &App, id: NodeId) -> Option<(f64, f64)> {
    app.with_state(|state| state.scene.views.get(&id)?.media.as_ref()?.position()).flatten()
}

/// Releases the player of a destroyed view.
// The scene owns it boxed and hands the box back.
#[allow(clippy::boxed_local)]
pub fn release(_app: &Rc<App>, mut player: Box<MediaPlayer>) {
    if let Some(engine) = player.detach() {
        shut_down(engine);
    }
}

fn error_text(code: usize, detail: u32) -> String {
    let reason = match code as i32 {
        value if value == MF_MEDIA_ENGINE_ERR_ABORTED.0 => "aborted",
        value if value == MF_MEDIA_ENGINE_ERR_NETWORK.0 => "network error",
        value if value == MF_MEDIA_ENGINE_ERR_DECODE.0 => "decoding error",
        value if value == MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED.0 => "source not supported",
        value if value == MF_MEDIA_ENGINE_ERR_ENCRYPTED.0 => "encrypted media",
        _ => "unknown error",
    };
    format!("Media playback failed ({reason}, 0x{detail:08X})")
}

/// An engine event, delivered on the UI thread.
pub fn on_engine_event(app: &Rc<App>, token: u64, event: u32, first: usize, second: u32) {
    let events = app.with_state(|state| {
        let id = state.scene.players.iter().copied().find(|id| state.scene.views.get(id).and_then(|view| view.media.as_ref()).is_some_and(|player| player.token == token && token != 0))?;
        let mut events: Vec<MediaEvent> = Vec::new();
        // Whether the picture or the controls may have changed.
        let mut repaint = false;
        let State { scene, graphics, .. } = state;
        let player = scene.views.get_mut(&id)?.media.as_deref_mut()?;
        let engine = player.engine.clone()?;
        let event = event as i32;
        // SAFETY: plain calls on the live engine; out pointers refer to live locals.
        unsafe {
            if event == MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA.0 {
                if !player.ready {
                    player.ready = true;
                    let (mut width, mut height) = (0_u32, 0_u32);
                    if engine.HasVideo().as_bool() {
                        let _ = engine.GetNativeVideoSize(Some(&mut width), Some(&mut height));
                    }
                    player.natural = (width, height);
                    let duration = engine.GetDuration();
                    player.duration = if duration.is_finite() { duration.max(0.0) } else { 0.0 };
                    if let Some(device) = graphics.device.as_ref() {
                        player.create_frame(device);
                    }
                    // After a device loss the application already knows the media.
                    let resumed = player.resume.take();
                    if resumed.is_none() {
                        events.push(MediaEvent::Ready { width, height, duration: player.duration });
                    }
                    let start = resumed.map_or(player.start_at, |(position, _)| position);
                    if start > 0.0 {
                        let _ = engine.SetCurrentTime(start);
                    }
                    if resumed.map_or(player.play_when_ready, |(_, playing)| playing) {
                        let _ = engine.Play();
                    }
                    player.awaiting_frame = FRAME_WAIT_TICKS;
                    repaint = true;
                }
            } else if event == MF_MEDIA_ENGINE_EVENT_LOADEDDATA.0 || event == MF_MEDIA_ENGINE_EVENT_FIRSTFRAMEREADY.0 || event == MF_MEDIA_ENGINE_EVENT_SEEKED.0 {
                player.awaiting_frame = FRAME_WAIT_TICKS;
                repaint = true;
            } else if event == MF_MEDIA_ENGINE_EVENT_PLAYING.0 {
                // `Playing` is only reported for the transport controls
                // (see `press`): echoing the application's own commands
                // back, late, would make it toggle them again.
                if std::mem::take(&mut player.buffering) {
                    events.push(MediaEvent::Buffering(false));
                }
                repaint = true;
            } else if event == MF_MEDIA_ENGINE_EVENT_PAUSE.0 {
                repaint = true;
            } else if event == MF_MEDIA_ENGINE_EVENT_WAITING.0 || event == MF_MEDIA_ENGINE_EVENT_BUFFERINGSTARTED.0 {
                if !std::mem::replace(&mut player.buffering, true) {
                    events.push(MediaEvent::Buffering(true));
                }
            } else if event == MF_MEDIA_ENGINE_EVENT_CANPLAY.0 || event == MF_MEDIA_ENGINE_EVENT_BUFFERINGENDED.0 {
                if std::mem::take(&mut player.buffering) {
                    events.push(MediaEvent::Buffering(false));
                }
            } else if event == MF_MEDIA_ENGINE_EVENT_ENDED.0 {
                // A looping engine restarts by itself and never ends.
                if !engine.GetLoop().as_bool() {
                    events.push(MediaEvent::Ended);
                }
                repaint = true;
            } else if event == MF_MEDIA_ENGINE_EVENT_ERROR.0 {
                // The engine of a lost device fails on its way out: that
                // player is being reopened, not broken.
                if player.resume.is_none() {
                    events.push(MediaEvent::Failed(error_text(first, second)));
                }
            }
        }
        if repaint {
            // Painting the player asks for the frames it needs.
            state.scene.damage_view(id);
        }
        Some((id, events))
    });
    if let Some(Some((id, events))) = events {
        for event in events {
            app.emit(BackendEvent::Media { id, event });
        }
    }
}

/// One vertical blank: players pull the frame that is due. Returns whether
/// any player needs the next blank too.
pub fn tick(state: &mut State) -> bool {
    let mut again = false;
    let mut index = 0;
    while let Some(id) = state.scene.players.get(index).copied() {
        index += 1;
        // A player nobody can see (hidden, or in a minimized window) pulls
        // no frames; painting it again resumes them.
        let minimized = state.scene.window_of(id).and_then(|window| state.scene.windows.get(&window)).is_none_or(|entry| entry.state.minimized);
        if minimized || !state.scene.is_shown(id) {
            continue;
        }
        let Some(player) = player_mut(state, id) else { continue };
        if !player.ready || player.engine.is_none() {
            continue;
        }
        let playing = player.is_playing();
        let mut changed = false;
        let mut first_frame = false;
        if player.texture.is_some() {
            if player.pull_frame() {
                player.has_frame = true;
                player.awaiting_frame = 0;
                first_frame = !std::mem::replace(&mut player.rendered, true);
                changed = true;
            } else if !playing && player.awaiting_frame > 0 {
                // Paused: the frame of a load or seek arrives a little later.
                player.awaiting_frame -= 1;
                again = true;
            }
            again |= playing;
        }
        // The seek bar follows playback while the controls show.
        if playing && player.controls_visible() {
            changed = true;
            again = true;
        }
        if first_frame {
            state.events.push(BackendEvent::Media { id, event: MediaEvent::Rendered });
        }
        if changed {
            state.scene.damage_view(id);
        }
    }
    again
}

/// The pointer moved: controls show on the player under it.
pub fn hover(state: &mut State, window: WindowId, position: Point) {
    let mut path = state.scene.windows.get_mut(&window).map(|entry| std::mem::take(&mut entry.path)).unwrap_or_default();
    if position.x >= 0.0 {
        if path.is_empty() {
            state.scene.hit_path(window, position, &mut path);
        }
    } else {
        path.clear();
    }
    let hovered = path.iter().rev().copied().find(|id| state.scene.views.get(id).is_some_and(|view| view.media.is_some()));
    let mut changed: Vec<NodeId> = Vec::new();
    for id in &state.scene.players {
        let Some(player) = state.scene.views.get_mut(id).and_then(|view| view.media.as_deref_mut()) else { continue };
        let now = hovered == Some(*id);
        if player.hovered != now {
            player.hovered = now;
            if player.controls {
                changed.push(*id);
            }
        }
    }
    if let Some(entry) = state.scene.windows.get_mut(&window) {
        entry.path = path;
    }
    if !changed.is_empty() {
        for id in changed {
            state.scene.damage_view(id);
        }
        state.request_tick();
    }
}

/// A primary press: the transport controls take it when it lands on them.
pub fn press(state: &mut State, path: &[NodeId], position: Point) -> bool {
    let Some(id) = path.iter().rev().copied().find(|id| state.scene.views.get(id).is_some_and(|view| view.media.is_some())) else { return false };
    let Some(local) = state.scene.world_transform(id).and_then(|(_, transform)| transform.invert()).map(|inverse| inverse.apply(position)) else { return false };
    let Some(view) = state.scene.views.get(&id) else { return false };
    let size = view.frame.size();
    let Some(player) = view.media.as_ref() else { return false };
    if !player.controls_visible() {
        return false;
    }
    let Some(layout) = ControlsLayout::new(size) else { return false };
    if !layout.bar.contains(local) {
        return false;
    }
    let Some(engine) = player.engine.clone() else { return true };
    let duration = player.duration;
    let mut toggled = None;
    // SAFETY: plain calls on the live engine.
    unsafe {
        if layout.play.contains(local) {
            if engine.IsPaused().as_bool() || engine.IsEnded().as_bool() {
                if engine.IsEnded().as_bool() {
                    let _ = engine.SetCurrentTime(0.0);
                }
                let _ = engine.Play();
                toggled = Some(true);
            } else {
                let _ = engine.Pause();
                toggled = Some(false);
            }
        } else if layout.mute.contains(local) {
            let _ = engine.SetMuted(!engine.GetMuted().as_bool());
        } else if local.x >= layout.track.x - 4.0 && local.x <= layout.track.right() + 4.0 && duration > 0.0 && layout.track.width > 0.0 {
            let fraction = ((local.x - layout.track.x) / layout.track.width).clamp(0.0, 1.0);
            let _ = engine.SetCurrentTime(duration * f64::from(fraction));
        }
    }
    if let Some(player) = player_mut(state, id) {
        player.awaiting_frame = FRAME_WAIT_TICKS;
        if let Some(playing) = toggled {
            player.play_when_ready = playing;
        }
    }
    if let Some(playing) = toggled {
        // The user played or paused with the controls: the application follows.
        state.events.push(BackendEvent::Media { id, event: MediaEvent::Playing(playing) });
    }
    state.scene.damage_view(id);
    state.request_tick();
    true
}

/// The graphics device was lost: frames and decoders died with it. The
/// engines are retired here and opened again by `recover`.
pub fn device_lost(state: &mut State) {
    state.media.manager = None;
    let mut retired = Vec::new();
    for id in &state.scene.players {
        let Some(player) = state.scene.views.get_mut(id).and_then(|view| view.media.as_deref_mut()) else { continue };
        if player.engine.is_none() {
            continue;
        }
        let resume = player.position().map_or((player.start_at, player.play_when_ready), |(time, _)| (time, player.is_playing()));
        if let Some(engine) = player.detach() {
            retired.push(engine);
        }
        player.resume = Some(resume);
    }
    state.media.retired.extend(retired);
}

/// Opens the players again after a device loss. Runs outside any state borrow.
pub fn recover(app: &Rc<App>) {
    let mailbox = Arc::clone(&app.mailbox);
    let outcome = app.with_state(|state| {
        let retired = std::mem::take(&mut state.media.retired);
        let mut failures = Vec::new();
        let State { scene, graphics, media: shared, .. } = state;
        for id in &scene.players {
            let Some(player) = scene.views.get_mut(id).and_then(|view| view.media.as_deref_mut()) else { continue };
            if player.resume.is_none() || player.engine.is_some() {
                continue;
            }
            let Some(source) = player.state.as_ref().and_then(|state| state.source.clone()) else { continue };
            match player.open(shared, graphics, &mailbox, &source) {
                Ok(()) => {
                    if let Some(wanted) = player.state.clone() {
                        player.apply_live(&wanted, None);
                    }
                }
                Err(message) => {
                    player.resume = None;
                    failures.push((*id, message));
                }
            }
        }
        (retired, failures)
    });
    let Some((retired, failures)) = outcome else { return };
    for engine in retired {
        shut_down(engine);
    }
    for (id, message) in failures {
        app.emit(BackendEvent::Media { id, event: MediaEvent::Failed(message) });
    }
}

/// Stops Media Foundation at application shutdown.
pub fn shutdown(app: &Rc<App>) {
    let engines = app.with_state(|state| {
        let mut engines = std::mem::take(&mut state.media.retired);
        for id in &state.scene.players {
            if let Some(engine) = state.scene.views.get_mut(id).and_then(|view| view.media.as_deref_mut()).and_then(MediaPlayer::detach) {
                engines.push(engine);
            }
        }
        state.media.factory = None;
        state.media.manager = None;
        (engines, std::mem::take(&mut state.media.started))
    });
    let Some((engines, started)) = engines else { return };
    for engine in engines {
        shut_down(engine);
    }
    if started {
        // SAFETY: balances the MFStartup of `MediaShared::prepare`, after
        // every engine was shut down.
        unsafe {
            let _ = MFShutdown();
        }
    }
}
