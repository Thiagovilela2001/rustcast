use bytes::Bytes;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, watch};

const BURST_BYTES: usize = 128 * 1024;

#[derive(Default)]
struct Burst {
    chunks: VecDeque<Bytes>,
    bytes: usize,
}

pub struct MountPoint {
    burst: Mutex<Burst>,
    tx: broadcast::Sender<Bytes>,
    pub meta: watch::Sender<String>,
    pub content_type: Mutex<String>,
    /// Media id now playing from AutoDJ (0 = none / live DJ). Used for cover art in nowplaying.
    pub media_id: AtomicI64,
    /// Unix secs when the current track / live set started.
    pub started_at: AtomicU64,
    live: AtomicBool,
    /// Admin controls for the AutoDJ (consumed by the engine / scheduler threads).
    pub skip: AtomicBool,
    pub requeue: Mutex<VecDeque<i64>>,
    /// Media ids announced on air, newest last (back stack for "previous").
    pub history: Mutex<VecDeque<i64>>,
    /// AutoDJ lookahead media id (0 = unknown).
    pub next_up: AtomicI64,
}

impl MountPoint {
    fn new() -> Self {
        Self {
            burst: Mutex::default(),
            tx: broadcast::channel(1024).0,
            meta: watch::channel(String::new()).0,
            content_type: Mutex::new("audio/mpeg".into()),
            media_id: AtomicI64::new(0),
            started_at: AtomicU64::new(0),
            live: AtomicBool::new(false),
            skip: AtomicBool::new(false),
            requeue: Mutex::default(),
            history: Mutex::default(),
            next_up: AtomicI64::new(0),
        }
    }

    /// Buffer + broadcast under one lock so a subscriber never misses or duplicates a chunk.
    pub fn publish(&self, chunk: Bytes) {
        let mut b = self.burst.lock().unwrap();
        b.bytes += chunk.len();
        b.chunks.push_back(chunk.clone());
        while b.bytes > BURST_BYTES {
            let old = b.chunks.pop_front().unwrap();
            b.bytes -= old.len();
        }
        let _ = self.tx.send(chunk);
    }

    pub fn subscribe(&self) -> (Vec<Bytes>, broadcast::Receiver<Bytes>) {
        let b = self.burst.lock().unwrap();
        (b.chunks.iter().cloned().collect(), self.tx.subscribe())
    }

    /// Announce what's on air (AutoDJ track or live DJ, media_id 0).
    pub fn set_now(&self, title: String, media_id: i64) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        self.started_at.store(now, Ordering::SeqCst);
        self.media_id.store(media_id, Ordering::SeqCst);
        if media_id > 0 {
            let mut h = self.history.lock().unwrap();
            h.push_back(media_id);
            if h.len() > 200 {
                h.pop_front();
            }
        }
        self.meta.send_replace(title);
    }

    /// Skip forward. False while a live DJ holds the mount.
    pub fn request_skip(&self) -> bool {
        if self.is_live() {
            return false;
        }
        self.skip.store(true, Ordering::SeqCst);
        true
    }

    /// Replay the previous track. Pops current + previous off the back stack, so pressing again goes further back.
    pub fn request_previous(&self) -> bool {
        if self.is_live() {
            return false;
        }
        let mut h = self.history.lock().unwrap();
        if h.len() < 2 {
            return false;
        }
        h.pop_back(); // current
        let prev = h.pop_back().unwrap(); // re-added when it goes on air
        drop(h);
        self.requeue.lock().unwrap().push_front(prev);
        self.skip.store(true, Ordering::SeqCst);
        true
    }

    pub fn is_live(&self) -> bool {
        self.live.load(Ordering::SeqCst)
    }

    pub fn listeners(&self) -> usize {
        self.tx.receiver_count()
    }

    /// One source per mount. Guard releases on drop (disconnect).
    pub fn claim(self: &Arc<Self>) -> Option<SourceGuard> {
        (!self.live.swap(true, Ordering::SeqCst)).then(|| SourceGuard(self.clone()))
    }
}

pub struct SourceGuard(Arc<MountPoint>);
impl Drop for SourceGuard {
    fn drop(&mut self) {
        self.0.live.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Default)]
pub struct Mounts(Arc<RwLock<HashMap<String, Arc<MountPoint>>>>);

impl Mounts {
    pub fn get(&self, name: &str) -> Option<Arc<MountPoint>> {
        self.0.read().unwrap().get(name).cloned()
    }
    pub fn get_or_create(&self, name: &str) -> Arc<MountPoint> {
        self.0
            .write()
            .unwrap()
            .entry(name.into())
            .or_insert_with(|| Arc::new(MountPoint::new()))
            .clone()
    }
}

/// "/mount/radio", "/radio", "radio" -> "radio"
pub fn normalize(path: &str) -> String {
    let p = path.trim_start_matches('/');
    p.strip_prefix("mount/").unwrap_or(p).to_string()
}
