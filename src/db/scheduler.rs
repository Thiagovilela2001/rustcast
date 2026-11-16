//! Picks the next track: weighted playlists, periodic jingles, artist anti-repeat. Reads DB every pick, so API edits apply at once.
use super::models::{Media, Playlist, Station, MEDIA_COLS, PLAYLIST_COLS};
use crate::audio::engine;
use crate::relay::mountpoint::Mounts;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NO_REPEAT_ARTISTS: usize = 3;

pub struct Scheduler {
    db: SqlitePool,
    station_id: i64,
    recent: VecDeque<String>,
    since_jingle: i64,
}

impl Scheduler {
    pub fn new(db: SqlitePool, station_id: i64) -> Self {
        Self { db, station_id, recent: VecDeque::new(), since_jingle: 0 }
    }

    pub async fn next(&mut self) -> sqlx::Result<Option<Media>> {
        let pls: Vec<Playlist> = sqlx::query_as(sql!(
            "SELECT {PLAYLIST_COLS} FROM playlists p WHERE station_id = ? AND EXISTS (SELECT 1 FROM playlist_media pm WHERE pm.playlist_id = p.id)"
        ))
        .bind(self.station_id)
        .fetch_all(&self.db)
        .await?;

        let due_jingle = pls
            .iter()
            .filter(|p| p.is_jingle && p.jingle_interval.unwrap_or(0) > 0 && self.since_jingle >= p.jingle_interval.unwrap_or(0))
            .min_by_key(|p| p.jingle_interval);
        let (playlist, is_jingle) = match due_jingle {
            Some(j) => (Some(j.id), true),
            None => (pick_weighted(pls.iter().filter(|p| !p.is_jingle)), false),
        };

        let candidates: Vec<Media> = match playlist {
            Some(pid) => sqlx::query_as(sql!(
                "SELECT {MEDIA_COLS} FROM media WHERE id IN (SELECT media_id FROM playlist_media WHERE playlist_id = ?)"
            ))
            .bind(pid),
            None => sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE station_id = ?")).bind(self.station_id),
        }
        .fetch_all(&self.db)
        .await?;

        // Avoid the last N artists; shrink N until something is left (small catalogs).
        let Some(pool) = (0..=self.recent.len()).rev().find_map(|k| {
            let skip = self.recent.iter().rev().take(k).collect::<Vec<_>>();
            let p: Vec<&Media> = candidates.iter().filter(|m| !skip.contains(&&m.artist)).collect();
            (!p.is_empty()).then_some(p)
        }) else {
            return Ok(None);
        };
        let m = pool[fastrand::usize(..pool.len())].clone();
        if is_jingle {
            self.since_jingle = 0;
        } else {
            self.since_jingle += 1;
            self.recent.push_back(m.artist.clone());
            if self.recent.len() > NO_REPEAT_ARTISTS {
                self.recent.pop_front();
            }
        }
        Ok(Some(m))
    }
}

fn pick_weighted<'a>(pls: impl Iterator<Item = &'a Playlist>) -> Option<i64> {
    let pls: Vec<_> = pls.filter(|p| p.weight > 0).collect();
    let total: i64 = pls.iter().map(|p| p.weight).sum();
    if total == 0 {
        return None;
    }
    let mut r = fastrand::i64(0..total);
    pls.into_iter().find(|p| {
        r -= p.weight;
        r < 0
    }).map(|p| p.id)
}

/// Spawn the AutoDJ thread for a station, fed by its scheduler. Call from inside the tokio runtime.
pub fn start_autodj(db: SqlitePool, mounts: &Mounts, st: &Station) {
    let rt = tokio::runtime::Handle::current();
    let mut sched = Scheduler::new(db, st.id);
    let mut idle_until = Instant::now();
    engine::spawn(mounts.get_or_create(&st.mountpoint), st.bitrate as u32, st.crossfade_secs as f32, move || {
        if Instant::now() < idle_until {
            return None; // empty catalog: don't hit the DB every 26 ms
        }
        match rt.block_on(sched.next()) {
            Ok(Some(m)) => Some((PathBuf::from(m.file_path), format!("{} - {}", m.artist, m.title), m.id)),
            other => {
                if let Err(e) = other {
                    eprintln!("scheduler: {e}");
                }
                idle_until = Instant::now() + Duration::from_secs(2);
                None
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_skips_zero_and_respects_weights() {
        let p = |id, weight| Playlist { id, station_id: 1, name: String::new(), weight, is_jingle: false, jingle_interval: None };
        let pls = [p(1, 0), p(2, 90), p(3, 10)];
        let mut hits = [0; 4];
        for _ in 0..10_000 {
            hits[pick_weighted(pls.iter()).unwrap() as usize] += 1;
        }
        assert_eq!(hits[1], 0);
        assert!(hits[2] > 8_500 && hits[3] > 600, "{hits:?}");
        assert_eq!(pick_weighted([p(1, 0)].iter()), None);
    }
}
