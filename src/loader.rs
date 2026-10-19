//! Background decoding: a small pool for full images and a LIFO pool for thumbnails,
//! so the thumbnails that were requested last (the ones on screen) load first.

use crate::decode::{self, Decoded};
use crate::meta::{self, Metadata};
use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui::{ColorImage, Context};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};

pub enum Msg {
    Full(PathBuf, Result<Decoded, String>),
    Thumb(PathBuf, u32, Result<ColorImage, String>),
    Meta(PathBuf, Metadata),
}

enum Job {
    Full(PathBuf),
    Meta(PathBuf),
}

struct Stack<T> {
    items: Mutex<Vec<T>>,
    cv: Condvar,
}

impl<T> Stack<T> {
    fn new() -> Self {
        Self { items: Mutex::new(Vec::new()), cv: Condvar::new() }
    }
    fn push(&self, item: T) {
        self.items.lock().unwrap().push(item);
        self.cv.notify_one();
    }
    fn pop(&self) -> T {
        let mut items = self.items.lock().unwrap();
        loop {
            if let Some(item) = items.pop() {
                return item;
            }
            items = self.cv.wait(items).unwrap();
        }
    }
    fn clear(&self) {
        self.items.lock().unwrap().clear();
    }
}

pub struct Loader {
    full: Arc<Stack<Job>>,
    thumbs: Arc<Stack<(PathBuf, u32)>>,
    wanted: Arc<Mutex<HashSet<PathBuf>>>,
    max_side: Arc<AtomicU32>,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
}

impl Loader {
    pub fn new(ctx: &Context) -> Self {
        let (tx, rx) = unbounded();
        let loader = Self {
            full: Arc::new(Stack::new()),
            thumbs: Arc::new(Stack::new()),
            wanted: Arc::new(Mutex::new(HashSet::new())),
            max_side: Arc::new(AtomicU32::new(8192)),
            tx,
            rx,
        };
        let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        for _ in 0..2 {
            let (jobs, wanted, max_side, tx, ctx) =
                (loader.full.clone(), loader.wanted.clone(), loader.max_side.clone(), loader.tx.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                match jobs.pop() {
                    Job::Full(path) => {
                        if !wanted.lock().unwrap().contains(&path) {
                            continue;
                        }
                        let side = max_side.load(Ordering::Relaxed);
                        let result = guard(|| decode::decode_full(&path, side));
                        let _ = tx.send(Msg::Full(path, result));
                        ctx.request_repaint();
                    }
                    Job::Meta(path) => {
                        let Ok(m) = std::panic::catch_unwind(|| meta::read_metadata(&path)) else { continue };
                        let _ = tx.send(Msg::Meta(path, m));
                        ctx.request_repaint();
                    }
                }
            });
        }
        for _ in 0..cpus.saturating_sub(2).clamp(2, 6) {
            let (jobs, tx, ctx) = (loader.thumbs.clone(), loader.tx.clone(), ctx.clone());
            std::thread::spawn(move || loop {
                let (path, size) = jobs.pop();
                let result = guard(|| decode::decode_thumb(&path, size));
                let _ = tx.send(Msg::Thumb(path, size, result));
                ctx.request_repaint();
            });
        }
        loader
    }

    pub fn set_max_side(&self, side: u32) {
        self.max_side.store(side.max(1024), Ordering::Relaxed);
    }

    /// Sets which full images are still worth decoding; stale queued jobs are skipped.
    pub fn set_wanted(&self, paths: impl IntoIterator<Item = PathBuf>) {
        let mut w = self.wanted.lock().unwrap();
        w.clear();
        w.extend(paths);
    }

    pub fn request_full(&self, path: PathBuf) {
        self.full.push(Job::Full(path));
    }

    pub fn request_meta(&self, path: PathBuf) {
        self.full.push(Job::Meta(path));
    }

    pub fn request_thumb(&self, path: PathBuf, size: u32) {
        self.thumbs.push((path, size));
    }

    pub fn clear_thumb_queue(&self) {
        self.thumbs.clear();
    }
}

/// A malformed file must not take the whole viewer down with it.
fn guard<T>(f: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe) -> Result<T, String> {
    std::panic::catch_unwind(f).unwrap_or_else(|_| Err("файл повреждён или не поддерживается".into()))
}
