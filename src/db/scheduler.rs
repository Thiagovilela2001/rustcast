//! Picks the next track: weighted playlists, periodic jingles, artist anti-repeat. Reads DB every pick, so API edits apply at once.
use super::models::{Media, Playlist, Station, MEDIA_COLS, PLAYLIST_COLS};
use crate::audio::engine;
use crate::relay::mountpoint::Mounts;
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NO_REPEAT_ARTISTS: usize = 3;
/// Track history kept for shuffle rotation (covers catalogs up to this size).
const HISTORY: usize = 2000;

pub struct Scheduler {
    db: SqlitePool,
    station_id: i64,
    recent: VecDeque<String>,
    played: VecDeque<i64>,
    since_jingle: i64,
}

impl Scheduler {
    pub fn new(db: SqlitePool, station_id: i64) -> Self {
        Self { db, station_id, recent: VecDeque::new(), played: VecDeque::new(), since_jingle: 0 }
    }

    /// Fetch a specific track (admin "previous"); it still counts in the shuffle history.
    pub async fn forced(&mut self, id: i64) -> sqlx::Result<Option<Media>> {
        let m: Option<Media> = sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE id = ? AND station_id = ?"))
            .bind(id)
            .bind(self.station_id)
            .fetch_optional(&self.db)
            .await?;
        if let Some(m) = &m {
            self.played.push_back(m.id);
        }
        Ok(m)
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

        let Some(m) = choose(&candidates, &self.played, &self.recent).cloned() else {
            return Ok(None);
        };
        self.played.push_back(m.id);
        if self.played.len() > HISTORY {
            self.played.pop_front();
        }
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

/// Shuffle rotation: never repeat a track until every other candidate played (window = n-1),
/// and avoid the last 3 artists ("Unknown" doesn't count). Rules relax, artist first, only if nothing is left.
fn choose<'a>(candidates: &'a [Media], played: &VecDeque<i64>, artists: &VecDeque<String>) -> Option<&'a Media> {
    let max_tw = candidates.len().saturating_sub(1).min(played.len());
    (0..=max_tw).rev().find_map(|tw| {
        let skip_ids: Vec<i64> = played.iter().rev().take(tw).copied().collect();
        let base: Vec<&Media> = candidates.iter().filter(|m| !skip_ids.contains(&m.id)).collect();
        let pool = (0..=artists.len()).rev().find_map(|k| {
            let skip: Vec<&String> = artists.iter().rev().take(k).collect();
            let p: Vec<&Media> = base.iter().copied().filter(|m| m.artist == "Unknown" || !skip.contains(&&m.artist)).collect();
            (!p.is_empty()).then_some(p)
        })?;
        Some(pool[fastrand::usize(..pool.len())])
    })
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
    let mount = mounts.get_or_create(&st.mountpoint);
    let ctl = mount.clone();
    engine::spawn(mount, st.bitrate as u32, st.crossfade_secs as f32, move || {
        let forced = ctl.requeue.lock().unwrap().pop_front();
        if let Some(id) = forced
            && let Ok(Some(m)) = rt.block_on(sched.forced(id))
        {
            return Some((PathBuf::from(m.file_path), format!("{} - {}", m.artist, m.title), m.id));
        }
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
    fn shuffle_plays_everything_before_repeating() {
        let media: Vec<Media> = (1..=41)
            .map(|id| Media { id, station_id: 1, file_path: String::new(), title: String::new(),
                artist: if id <= 8 { "Unknown".into() } else { format!("A{}", id % 15) }, album: None, duration_ms: 0 })
            .collect();
        let (mut played, mut artists) = (VecDeque::new(), VecDeque::new());
        for round in 0..3 {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..media.len() {
                let m = choose(&media, &played, &artists).unwrap();
                assert!(seen.insert(m.id), "round {round}: {} repeated", m.id);
                played.push_back(m.id);
                artists.push_back(m.artist.clone());
                if artists.len() > NO_REPEAT_ARTISTS { artists.pop_front(); }
            }
        }
        let one = [media[0].clone()];
        assert_eq!(choose(&one, &played, &artists).unwrap().id, 1); // single track still plays
    }

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
