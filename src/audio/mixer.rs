//! Two-deck mixer: equal-power crossfade over the last N seconds of the current track, or a short one on skip.
use super::deck::{Deck, RATE};
use std::f32::consts::FRAC_PI_2;

const SKIP_FADE_SECS: f32 = 0.5;

pub struct Mixer {
    current: Option<Deck>,
    next: Option<Deck>,
    fade_frames: u64,
    fade_pos: u64,
    fade_len: u64,
    b: Vec<f32>,
}

impl Mixer {
    pub fn new(crossfade_secs: f32) -> Self {
        Self {
            current: None,
            next: None,
            fade_frames: (crossfade_secs * RATE as f32) as u64,
            fade_pos: 0,
            fade_len: 1,
            b: Vec::new(),
        }
    }

    /// Fill `out` (interleaved stereo). Silence when nothing to play. Returns new title when a track takes over.
    pub fn fill(
        &mut self,
        out: &mut [f32],
        open_next: &mut dyn FnMut() -> Option<Deck>,
    ) -> Option<(String, i64)> {
        let mut title = None;
        if self.current.is_none() {
            self.current = open_next();
            title = self.current.as_ref().map(|d| (d.title.clone(), d.media_id));
        }
        let Some(cur) = &mut self.current else {
            out.fill(0.0);
            return None;
        };
        if self.next.is_none()
            && self.fade_frames > 0
            && let Some(rem) = cur.remaining_frames()
            && rem <= self.fade_frames
        {
            self.next = open_next();
            self.fade_pos = 0;
            self.fade_len = rem.max(1);
        }
        let got = cur.read(out);
        if let Some(nx) = &mut self.next {
            self.b.resize(out.len(), 0.0);
            nx.read(&mut self.b);
            for (i, (a, b)) in out
                .chunks_exact_mut(2)
                .zip(self.b.chunks_exact(2))
                .enumerate()
            {
                let t =
                    ((self.fade_pos + i as u64) as f32 / self.fade_len as f32).min(1.0) * FRAC_PI_2;
                let (ga, gb) = (t.cos(), t.sin());
                a[0] = a[0] * ga + b[0] * gb;
                a[1] = a[1] * ga + b[1] * gb;
            }
            self.fade_pos += (out.len() / 2) as u64;
        }
        // Track over, or (skip) the short fade finished before the track did.
        if got < out.len() / 2 || (self.next.is_some() && self.fade_pos >= self.fade_len) {
            // current over: next deck (if any) takes over; else next block opens a fresh one
            self.current = self.next.take();
            title = self.current.as_ref().map(|d| (d.title.clone(), d.media_id));
        }
        title
    }

    /// Jump to the next track with a short crossfade. Returns a title only when it takes over immediately.
    pub fn skip(&mut self, open_next: &mut dyn FnMut() -> Option<Deck>) -> Option<(String, i64)> {
        // Mid-crossfade the listener already hears the incoming track as "current": drop the outgoing one, skip that.
        if self.next.is_some() {
            self.current = self.next.take();
        }
        let next = open_next();
        match (&self.current, next) {
            (Some(_), Some(n)) => {
                // title is announced by fill() when the fade completes and the deck takes over (once)
                self.next = Some(n);
                self.fade_pos = 0;
                self.fade_len = (SKIP_FADE_SECS * RATE as f32) as u64;
                None
            }
            (_, n) => {
                self.current = n;
                self.current.as_ref().map(|d| (d.title.clone(), d.media_id))
            }
        }
    }
}
