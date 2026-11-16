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
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.started_at.store(now, Ordering::SeqCst);
        self.media_id.store(media_id, Ordering::SeqCst);
        self.meta.send_replace(title);
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
        self.0.write().unwrap().entry(name.into()).or_insert_with(|| Arc::new(MountPoint::new())).clone()
    }
}

/// "/mount/radio", "/radio", "radio" -> "radio"
pub fn normalize(path: &str) -> String {
    let p = path.trim_start_matches('/');
    p.strip_prefix("mount/").unwrap_or(p).to_string()
}
