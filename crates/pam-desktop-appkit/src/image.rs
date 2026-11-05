//! Image decoding off the main thread: ImageIO for pictures, `NSImage`
//! for SVG documents, which ImageIO does not read.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use objc2::AnyThread;
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSImage;
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL,
};
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSData, NSPoint, NSRect, NSSize};
use objc2_image_io::{
    CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately,
    kCGImageSourceThumbnailMaxPixelSize,
};
use pam_desktop_ui::backend::{BackendEvent, ImageKey, ImageSource};
use pam_desktop_ui::geometry::Size;

use crate::events::{Events, MainThread};

/// Threads that decode; they sleep on the queue while there is no work.
const WORKERS: usize = 2;

struct Job {
    key: ImageKey,
    source: ImageSource,
    max_dimension: Option<u32>,
}

/// Decoded images, by key, and the decodes in flight.
pub struct ImageStore {
    events: Rc<Events>,
    images: RefCell<HashMap<ImageKey, CFRetained<CGImage>>>,
    /// Keys being decoded; a key released meanwhile is dropped on arrival.
    pending: RefCell<HashSet<ImageKey>>,
    queue: RefCell<Option<mpsc::Sender<Job>>>,
}

thread_local! {
    /// The store of the running backend: decoded images arrive from other
    /// threads, which cannot carry a reference to it.
    static STORE: RefCell<Option<Rc<ImageStore>>> = const { RefCell::new(None) };
}

/// The largest side an SVG is drawn at, in pixels.
const SVG_LIMIT: f64 = 4096.0;

/// Whether `bytes` start an SVG document: the root element, possibly after
/// a byte order mark, an XML declaration, a doctype or comments.
fn is_svg(bytes: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(2048)]);
    let head = head.trim_start_matches('\u{feff}').trim_start();
    head.starts_with("<svg")
        || (["<?xml", "<!--", "<!DOCTYPE", "<!doctype"]
            .iter()
            .any(|prefix| head.starts_with(prefix))
            && head.contains("<svg"))
}

/// The pixel size an SVG of `natural` size is drawn at. A drawing has no
/// pixels of its own: it is drawn at the size it will be shown at
/// (`max_dimension`), or at twice its natural size when that is unknown, so
/// that it stays sharp on a dense display.
fn svg_pixels(natural: (f64, f64), max_dimension: Option<u32>) -> Option<(f64, f64)> {
    let (width, height) = natural;
    if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) {
        return None;
    }
    let larger = width.max(height);
    let wanted = max_dimension
        .filter(|limit| *limit > 0)
        .map_or(larger * 2.0, f64::from)
        .min(SVG_LIMIT);
    let scale = wanted / larger;
    Some((
        (width * scale).round().max(1.0),
        (height * scale).round().max(1.0),
    ))
}

/// Draws an SVG document into pixels on the calling (worker) thread.
/// `NSImage` reads SVG from macOS 10.15 on.
fn decode_svg(bytes: &[u8], max_dimension: Option<u32>) -> Result<CFRetained<CGImage>, String> {
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))
        .ok_or_else(|| String::from("Image could not be decoded"))?;
    let natural = image.size();
    let (width, height) = svg_pixels((natural.width, natural.height), max_dimension)
        .ok_or_else(|| String::from("Image could not be decoded"))?;
    let mut area = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(width, height));
    // SAFETY: the rectangle is a valid pointer for the call; without a
    // context it is taken in pixels, and no hints are passed.
    unsafe { image.CGImageForProposedRect_context_hints(&mut area, None, None) }
        .map(CFRetained::from)
        .ok_or_else(|| String::from("Image format is not supported"))
}

/// Decodes on the calling (worker) thread. The image always goes through
/// ImageIO's thumbnail path: it decodes straight to the wanted size, so
/// large photos never exist at full size in memory, applies the
/// orientation the file records, and yields pixels that are ready to draw.
fn decode(source: &ImageSource, max_dimension: Option<u32>) -> Result<CFRetained<CGImage>, String> {
    match source {
        ImageSource::Bytes(bytes) if is_svg(bytes) => return decode_svg(bytes, max_dimension),
        ImageSource::File(path)
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("svg")) =>
        {
            let bytes =
                std::fs::read(path).map_err(|_| String::from("Image file could not be read"))?;
            return decode_svg(&bytes, max_dimension);
        }
        _ => {}
    }
    // SAFETY: the data, the address and the options outlive the calls, and
    // the option values have the types ImageIO documents for their keys.
    unsafe {
        let native = match source {
            ImageSource::Bytes(bytes) => CGImageSource::with_data(&CFData::from_bytes(bytes), None),
            ImageSource::File(path) => {
                if !path.is_file() {
                    return Err(String::from("Image file could not be read"));
                }
                CFURL::from_file_path(path).and_then(|url| CGImageSource::with_url(&url, None))
            }
        }
        .ok_or_else(|| String::from("Image could not be decoded"))?;
        let yes: &CFType = CFBoolean::new(true).as_ref();
        let mut keys: Vec<&CFString> = vec![
            kCGImageSourceCreateThumbnailFromImageAlways,
            kCGImageSourceCreateThumbnailWithTransform,
            kCGImageSourceShouldCacheImmediately,
        ];
        let mut values: Vec<&CFType> = vec![yes, yes, yes];
        let limit = max_dimension
            .filter(|limit| *limit > 0)
            .map(|limit| CFNumber::new_i64(i64::from(limit)));
        if let Some(limit) = &limit {
            keys.push(kCGImageSourceThumbnailMaxPixelSize);
            values.push(limit.as_ref());
        }
        let options: CFRetained<CFDictionary<CFString, CFType>> =
            CFDictionary::from_slices(&keys, &values);
        native
            .thumbnail_at_index(0, Some(options.as_ref()))
            .ok_or_else(|| String::from("Image format is not supported"))
    }
}

/// Runs on the main thread with the result of a decode.
fn deliver(key: ImageKey, result: Result<CFRetained<CGImage>, String>) {
    let Some(store) = STORE.with(|store| store.borrow().clone()) else {
        return;
    };
    if !store.pending.borrow_mut().remove(&key) {
        // Released while it was decoding: nobody waits for it.
        return;
    }
    match result {
        Ok(image) => {
            let size = Size::new(
                CGImage::width(Some(&image)) as f32,
                CGImage::height(Some(&image)) as f32,
            );
            store.images.borrow_mut().insert(key, image);
            store.events.push(BackendEvent::ImageLoaded { key, size });
        }
        Err(message) => store
            .events
            .push(BackendEvent::ImageFailed { key, message }),
    }
}

fn start_workers(main: &MainThread) -> Option<mpsc::Sender<Job>> {
    let (sender, receiver) = mpsc::channel::<Job>();
    let receiver = Arc::new(Mutex::new(receiver));
    let mut started = 0;
    for _ in 0..WORKERS {
        let (receiver, main) = (Arc::clone(&receiver), main.clone());
        let spawned = std::thread::Builder::new()
            .name("pam-image-decode".into())
            .spawn(move || {
                loop {
                    // The lock is only held while waiting for a job.
                    let job = match receiver.lock() {
                        Ok(receiver) => receiver.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else { return };
                    // Foundation objects made while decoding die with the pool.
                    let result = autoreleasepool(|_| decode(&job.source, job.max_dimension));
                    main.run(move || deliver(job.key, result));
                }
            });
        started += usize::from(spawned.is_ok());
    }
    (started > 0).then_some(sender)
}

impl ImageStore {
    /// Creates the store and makes it the one decodes are delivered to.
    pub fn install(events: &Rc<Events>) -> Rc<Self> {
        let store = Rc::new(Self {
            events: Rc::clone(events),
            images: RefCell::new(HashMap::new()),
            pending: RefCell::new(HashSet::new()),
            queue: RefCell::new(None),
        });
        STORE.with(|current| *current.borrow_mut() = Some(Rc::clone(&store)));
        store
    }

    /// Stops the workers and forgets the store.
    pub fn uninstall(&self) {
        self.queue.borrow_mut().take();
        STORE.with(|current| current.borrow_mut().take());
    }

    pub fn load(&self, key: ImageKey, source: ImageSource, max_dimension: Option<u32>) {
        let mut queue = self.queue.borrow_mut();
        if queue.is_none() {
            *queue = start_workers(self.events.main());
        }
        self.pending.borrow_mut().insert(key);
        let sent = queue.as_ref().is_some_and(|queue| {
            queue
                .send(Job {
                    key,
                    source,
                    max_dimension,
                })
                .is_ok()
        });
        drop(queue);
        if !sent {
            self.pending.borrow_mut().remove(&key);
            self.events.push(BackendEvent::ImageFailed {
                key,
                message: "image decoder could not start".into(),
            });
        }
    }

    pub fn release(&self, key: ImageKey) {
        self.pending.borrow_mut().remove(&key);
        self.images.borrow_mut().remove(&key);
    }

    pub fn get(&self, key: ImageKey) -> Option<CFRetained<CGImage>> {
        self.images.borrow().get(&key).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_documents_are_recognised_by_their_root() {
        assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
        assert!(is_svg(
            b"\xEF\xBB\xBF  <?xml version=\"1.0\"?>\n<!-- logo -->\n<svg/>"
        ));
        assert!(is_svg(
            b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"\">\n<svg/>"
        ));
        assert!(!is_svg(b"\x89PNG\r\n\x1a\n"));
        assert!(!is_svg(b"<?xml version=\"1.0\"?><plist/>"));
        assert!(!is_svg(b""));
    }

    #[test]
    fn svg_is_drawn_at_the_size_it_is_shown_at() {
        // The larger side takes the wanted size; the ratio is kept.
        assert_eq!(svg_pixels((24.0, 12.0), Some(96)), Some((96.0, 48.0)));
        // Unknown: twice the natural size.
        assert_eq!(svg_pixels((24.0, 24.0), None), Some((48.0, 48.0)));
        assert_eq!(svg_pixels((24.0, 24.0), Some(0)), Some((48.0, 48.0)));
        // Never larger than the limit.
        assert_eq!(svg_pixels((8000.0, 4000.0), None), Some((4096.0, 2048.0)));
        assert_eq!(svg_pixels((0.0, 10.0), Some(64)), None);
    }
}
