//! Image thumbnails, decoded off the UI thread.

use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

const MAX_SIDE: u32 = 320;
const MAX_BYTES: u64 = 60 << 20;

struct Job {
    generation: u64,
    path: PathBuf,
}

pub struct Thumbs {
    tx: Sender<Job>,
    pub generation: Arc<AtomicU64>,
}

impl Thumbs {
    /// `done` runs on a worker thread with (generation, path, pixels).
    pub fn new(done: impl Fn(u64, PathBuf, Pixels) + Send + Sync + 'static) -> Self {
        let (tx, rx) = channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let generation = Arc::new(AtomicU64::new(0));
        let done = Arc::new(done);
        for _ in 0..2 {
            let (rx, generation, done) = (rx.clone(), generation.clone(), done.clone());
            std::thread::spawn(move || worker(&rx, &generation, &*done));
        }
        Thumbs { tx, generation }
    }

    /// Drop everything queued so far (the folder changed).
    pub fn bump(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn request(&self, generation: u64, path: PathBuf) {
        let _ = self.tx.send(Job { generation, path });
    }
}

fn worker(rx: &Mutex<Receiver<Job>>, generation: &AtomicU64, done: &(dyn Fn(u64, PathBuf, Pixels) + Send + Sync)) {
    loop {
        let job = match rx.lock().unwrap().recv() {
            Ok(j) => j,
            Err(_) => return,
        };
        if job.generation != generation.load(Ordering::SeqCst) {
            continue;
        }
        if let Some(px) = decode(&job.path) {
            done(job.generation, job.path, px);
        }
    }
}

fn decode(path: &PathBuf) -> Option<Pixels> {
    if std::fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let img = img.thumbnail(MAX_SIDE, MAX_SIDE).to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

pub fn supported(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}
