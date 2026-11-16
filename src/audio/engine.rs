//! Real-time AutoDJ loop on a dedicated thread: 1152 frames every ~26.12 ms, encoded to MP3, published to a mount.
//! While a live DJ holds the mount, the AutoDJ freezes (decks keep their position, queue not consumed)
//! and resumes with a short fade-in when the DJ disconnects.
use super::{deck::{Deck, RATE}, encoder::Mp3, mixer::Mixer};
use crate::relay::mountpoint::MountPoint;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BLOCK: usize = 1152;
const RESUME_FADE_SECS: f32 = 1.0;

pub fn spawn(
    mount: Arc<MountPoint>,
    kbps: u32,
    crossfade_secs: f32,
    mut next_track: impl FnMut() -> Option<(PathBuf, String, i64)> + Send + 'static,
) {
    std::thread::spawn(move || {
        let mut mixer = Mixer::new(crossfade_secs);
        let mut enc = Mp3::new(kbps);
        let mut buf = vec![0f32; BLOCK * 2];
        // Try a few paths per call so one bad file does not stall the queue.
        let mut open_next = move || {
            (0..8).find_map(|_| {
                let (p, title, id) = next_track()?;
                let mut d = Deck::open(&p).map_err(|e| eprintln!("autodj: skip {}: {e}", p.display())).ok()?;
                d.title = title;
                d.media_id = id;
                Some(d)
            })
        };
        let announce = |t: &(String, i64)| {
            mount.media_id.store(t.1, Ordering::SeqCst);
            mount.meta.send_replace(t.0.clone());
        };
        let tick = Duration::from_secs_f64(BLOCK as f64 / RATE as f64);
        let fade_step = 1.0 / (RESUME_FADE_SECS * RATE as f32);
        let mut deadline = Instant::now();
        let mut now_playing: Option<(String, i64)> = None;
        let mut paused = false;
        let mut gain = 1.0f32;
        loop {
            if mount.is_live() {
                paused = true;
            } else {
                if paused {
                    // DJ left: take the mount back, fade in from where we stopped
                    paused = false;
                    gain = 0.0;
                    if let Some(t) = &now_playing {
                        announce(t);
                    }
                }
                if let Some(t) = mixer.fill(&mut buf, &mut open_next) {
                    announce(&t);
                    now_playing = Some(t);
                }
                if gain < 1.0 {
                    for f in buf.chunks_exact_mut(2) {
                        gain = (gain + fade_step).min(1.0);
                        f[0] *= gain;
                        f[1] *= gain;
                    }
                }
                let mp3 = enc.encode(&buf);
                // ponytail: tiny race if a DJ claims between checks; a few stray MP3 frames, decoders resync
                if !mp3.is_empty() && !mount.is_live() {
                    mount.publish(mp3);
                }
            }
            deadline += tick;
            let now = Instant::now();
            if deadline > now {
                std::thread::sleep(deadline - now);
            } else if now - deadline > Duration::from_secs(1) {
                deadline = now; // fell far behind (suspend, overload): resync instead of bursting
            }
        }
    });
}
