//! Decoded images: WIC decoding on worker threads, Direct2D bitmaps on the
//! UI thread, keyed by the core's `ImageKey`.
//!
//! Only the bitmap lives in memory (on the GPU). The encoded source is kept
//! by reference so a lost device decodes it again.

use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use pam_desktop_ui::backend::{BackendEvent, ImageKey, ImageSource};
use pam_desktop_ui::geometry::Size;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICBitmapSource, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeMedianCut,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::core::{Interface, PCWSTR};

use crate::app::{App, Mail, Mailbox};

/// Longest side accepted for a decoded image, in pixels.
const MAX_SIDE: u32 = 16_384;
/// Decoded size limit, in bytes of premultiplied BGRA.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
const WORKERS: usize = 2;

/// Premultiplied BGRA pixels of a decoded image.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
}

/// What a worker hands back to the UI thread.
pub struct Decoded {
    key: ImageKey,
    serial: u64,
    /// Report `ImageLoaded` / `ImageFailed` (not a reload after a device loss).
    announce: bool,
    result: Result<Pixels, String>,
}

struct Job {
    key: ImageKey,
    serial: u64,
    announce: bool,
    source: ImageSource,
    max_dimension: Option<u32>,
    mailbox: Arc<Mailbox>,
}

struct Entry {
    bitmap: Option<ID2D1Bitmap1>,
    source: ImageSource,
    max_dimension: Option<u32>,
    /// Distinguishes this load from an earlier one of the same key.
    serial: u64,
}

#[derive(Default)]
pub struct Images {
    entries: HashMap<ImageKey, Entry>,
    next_serial: u64,
    jobs: Option<Sender<Job>>,
    mailbox: Option<Arc<Mailbox>>,
}

fn decode(
    factory: &IWICImagingFactory,
    source: &ImageSource,
    max_dimension: Option<u32>,
) -> Result<Pixels, String> {
    if crate::svg::is_svg(source) {
        return crate::svg::rasterize(factory, &crate::svg::read(source)?, max_dimension);
    }
    let failed =
        |error: windows::core::Error| format!("Image could not be decoded ({})", error.message());
    // SAFETY: COM calls on objects created in this function. The memory
    // stream borrows `bytes`, which outlives every object reading from it.
    unsafe {
        let decoder = match source {
            ImageSource::File(path) => {
                let wide: Vec<u16> = path
                    .as_os_str()
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect();
                factory
                    .CreateDecoderFromFilename(
                        PCWSTR(wide.as_ptr()),
                        None,
                        GENERIC_READ,
                        WICDecodeMetadataCacheOnDemand,
                    )
                    .map_err(|error| {
                        format!("Image file could not be read ({})", error.message())
                    })?
            }
            ImageSource::Bytes(bytes) => {
                let stream = factory.CreateStream().map_err(failed)?;
                stream.InitializeFromMemory(bytes).map_err(failed)?;
                factory
                    .CreateDecoderFromStream(
                        &stream,
                        std::ptr::null(),
                        WICDecodeMetadataCacheOnDemand,
                    )
                    .map_err(failed)?
            }
        };
        let frame = decoder.GetFrame(0).map_err(failed)?;
        let (mut width, mut height) = (0_u32, 0_u32);
        frame.GetSize(&mut width, &mut height).map_err(failed)?;
        if width == 0 || height == 0 {
            return Err("Image is empty".into());
        }
        let longest = width.max(height);
        let limit = max_dimension
            .filter(|limit| *limit > 0)
            .unwrap_or(MAX_SIDE)
            .min(MAX_SIDE);
        let mut pixels: IWICBitmapSource = frame.cast().map_err(failed)?;
        if longest > limit {
            // The scaler asks the decoder to shrink while decoding when the
            // codec can (JPEG), so a large photo never exists at full size.
            let ratio = f64::from(limit) / f64::from(longest);
            width = ((f64::from(width) * ratio).round() as u32).max(1);
            height = ((f64::from(height) * ratio).round() as u32).max(1);
            let scaler = factory.CreateBitmapScaler().map_err(failed)?;
            scaler
                .Initialize(&pixels, width, height, WICBitmapInterpolationModeFant)
                .map_err(failed)?;
            pixels = scaler.cast().map_err(failed)?;
        }
        let stride = width
            .checked_mul(4)
            .ok_or_else(|| String::from("Image is too large"))?;
        let bytes = u64::from(stride) * u64::from(height);
        if bytes > MAX_BYTES {
            return Err("Image is too large".into());
        }
        let converter = factory.CreateFormatConverter().map_err(failed)?;
        converter
            .Initialize(
                &pixels,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeMedianCut,
            )
            .map_err(failed)?;
        let mut data = vec![0_u8; bytes as usize];
        converter
            .CopyPixels(std::ptr::null(), stride, &mut data)
            .map_err(failed)?;
        Ok(Pixels {
            width,
            height,
            stride,
            data,
        })
    }
}

fn worker(jobs: &Mutex<Receiver<Job>>) {
    // SAFETY: COM is initialized for this thread before any COM call and
    // uninitialized after the last object is released.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    {
        // SAFETY: standard in-process activation of the WIC factory.
        let factory: Option<IWICImagingFactory> =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok() };
        loop {
            // The lock is held only while waiting for a job, never while decoding.
            let job = match jobs.lock() {
                Ok(receiver) => receiver.recv(),
                Err(_) => break,
            };
            let Ok(job) = job else { break };
            let result = match &factory {
                Some(factory) => decode(factory, &job.source, job.max_dimension),
                None => Err(String::from(
                    "The Windows Imaging Component is not available",
                )),
            };
            job.mailbox.post(Mail::Image(Decoded {
                key: job.key,
                serial: job.serial,
                announce: job.announce,
                result,
            }));
        }
    }
    if initialized {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
}

impl Images {
    fn sender(&mut self) -> Option<&Sender<Job>> {
        if self.jobs.is_none() {
            let (sender, receiver) = channel::<Job>();
            let receiver = Arc::new(Mutex::new(receiver));
            let mut started = 0;
            for index in 0..WORKERS {
                let receiver = Arc::clone(&receiver);
                let spawned = std::thread::Builder::new()
                    .name(format!("pam-image-decode-{index}"))
                    .spawn(move || worker(&receiver));
                started += usize::from(spawned.is_ok());
            }
            if started == 0 {
                return None;
            }
            self.jobs = Some(sender);
        }
        self.jobs.as_ref()
    }

    /// Starts decoding. Returns `false` when no worker could start.
    pub fn load(
        &mut self,
        mailbox: &Arc<Mailbox>,
        key: ImageKey,
        source: ImageSource,
        max_dimension: Option<u32>,
    ) -> bool {
        self.next_serial += 1;
        let serial = self.next_serial;
        self.mailbox = Some(Arc::clone(mailbox));
        self.entries.insert(
            key,
            Entry {
                bitmap: None,
                source: source.clone(),
                max_dimension,
                serial,
            },
        );
        let job = Job {
            key,
            serial,
            announce: true,
            source,
            max_dimension,
            mailbox: Arc::clone(mailbox),
        };
        let sent = self.sender().is_some_and(|sender| sender.send(job).is_ok());
        if !sent {
            self.entries.remove(&key);
        }
        sent
    }

    pub fn release(&mut self, key: ImageKey) {
        self.entries.remove(&key);
    }

    pub fn bitmap(&self, key: ImageKey) -> Option<&ID2D1Bitmap1> {
        self.entries.get(&key)?.bitmap.as_ref()
    }

    /// Bitmaps died with the device: decode every image again, silently.
    pub fn device_lost(&mut self) {
        let Some(mailbox) = self.mailbox.clone() else {
            return;
        };
        let mut jobs = Vec::with_capacity(self.entries.len());
        for (key, entry) in &mut self.entries {
            if entry.bitmap.take().is_some() {
                jobs.push(Job {
                    key: *key,
                    serial: entry.serial,
                    announce: false,
                    source: entry.source.clone(),
                    max_dimension: entry.max_dimension,
                    mailbox: Arc::clone(&mailbox),
                });
            }
        }
        if let Some(sender) = self.sender() {
            for job in jobs {
                let _ = sender.send(job);
            }
        }
    }
}

/// A worker finished: the pixels become a bitmap and the core is told.
pub fn decoded(app: &Rc<App>, decoded: Decoded) {
    let Decoded {
        key,
        serial,
        announce,
        result,
    } = decoded;
    let event = app.with_state(|state| {
        // Released or loaded again meanwhile: the result is stale.
        if state
            .images
            .entries
            .get(&key)
            .is_none_or(|entry| entry.serial != serial)
        {
            return None;
        }
        let outcome = result.and_then(|pixels| {
            state.graphics.ensure_device();
            let device = state
                .graphics
                .device
                .as_ref()
                .ok_or_else(|| String::from("No graphics device is available"))?;
            let bitmap = device
                .create_pixel_bitmap(pixels.width, pixels.height, pixels.stride, &pixels.data)
                .map_err(|error| format!("Image could not be uploaded ({})", error.message()))?;
            Ok((bitmap, Size::new(pixels.width as f32, pixels.height as f32)))
        });
        match outcome {
            Ok((bitmap, size)) => {
                if let Some(entry) = state.images.entries.get_mut(&key) {
                    entry.bitmap = Some(bitmap);
                }
                // Views showing the key repaint (after a device loss they
                // were painted without it).
                let windows: Vec<_> = state.scene.windows.keys().copied().collect();
                for window in windows {
                    state.scene.damage_all(window);
                }
                announce.then_some(BackendEvent::ImageLoaded { key, size })
            }
            Err(message) => {
                if announce {
                    state.images.entries.remove(&key);
                }
                announce.then_some(BackendEvent::ImageFailed { key, message })
            }
        }
    });
    if let Some(Some(event)) = event {
        app.emit(event);
    }
}
