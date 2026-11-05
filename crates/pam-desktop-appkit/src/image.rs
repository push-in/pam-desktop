//! Image decoding with ImageIO, off the main thread.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};

use objc2::rc::autoreleasepool;
use objc2_core_foundation::{
    CFBoolean, CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL,
};
use objc2_core_graphics::CGImage;
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

/// Decodes on the calling (worker) thread. The image always goes through
/// ImageIO's thumbnail path: it decodes straight to the wanted size, so
/// large photos never exist at full size in memory, applies the
/// orientation the file records, and yields pixels that are ready to draw.
fn decode(source: &ImageSource, max_dimension: Option<u32>) -> Result<CFRetained<CGImage>, String> {
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
