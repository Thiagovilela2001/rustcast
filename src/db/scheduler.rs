//! Picks the next track. Behaviour modelled on AzuraCast's AutoDJ (clean reimplementation, no code reused):
//! playlist types by priority (once per hour > once per X songs > once per X minutes > default by weight),
//! orders shuffle / random / sequential, artist anti-repeat. Reads the DB every pick, so API edits apply at once.
use super::models::{MEDIA_COLS, Media, PLAYLIST_COLS, Playlist, Station};
use super::schedule::{SCHEDULE_COLS, Schedule};
use crate::audio::engine;
use crate::relay::mountpoint::Mounts;
use sqlx::SqlitePool;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

const NO_REPEAT_ARTISTS: usize = 3;
/// Track history kept for shuffle rotation (covers catalogs up to this size).
const HISTORY: usize = 2000;
/// Station local time offset from UTC in minutes (e.g. -180 for Brasília). Set once at startup.
pub static UTC_OFFSET_MIN: AtomicI64 = AtomicI64::new(0);
const TYPES_BY_PRIORITY: [&str; 4] = [
    "once_per_hour",
    "once_per_x_songs",
    "once_per_x_minutes",
    "default",
];

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// Once-per-hour rule: due from minute `target` for 15 minutes, if not played in the last 30.
fn per_hour_due(now: i64, offset_min: i64, target: i64, played_at: i64) -> bool {
    let minute = (now + offset_min * 60).rem_euclid(3600) / 60;
    (minute - target).rem_euclid(60) <= 15 && now - played_at >= 30 * 60
}

pub struct Scheduler {
    db: SqlitePool,
    station_id: i64,
    recent: VecDeque<String>,
    played: VecDeque<i64>,
    /// Next track, picked one ahead so "playing next" is known. Only one Media kept in RAM.
    lookahead: Option<Media>,
    loaded: bool,
    /// Songs picked since each playlist last played (once_per_x_songs). Resets on restart.
    since: HashMap<i64, i64>,
}

impl Scheduler {
    pub fn new(db: SqlitePool, station_id: i64) -> Self {
        Self {
            db,
            station_id,
            recent: VecDeque::new(),
            played: VecDeque::new(),
            lookahead: None,
            loaded: false,
            since: HashMap::new(),
        }
    }

    /// Fetch a specific track (admin "previous"); it still counts in the shuffle history.
    pub async fn forced(&mut self, id: i64) -> sqlx::Result<Option<Media>> {
        let m: Option<Media> = sqlx::query_as(sql!(
            "SELECT {MEDIA_COLS} FROM media WHERE id = ? AND station_id = ?"
        ))
        .bind(id)
        .bind(self.station_id)
        .fetch_optional(&self.db)
        .await?;
        if let Some(m) = &m {
            self.played.push_back(m.id);
        }
        Ok(m)
    }

    /// Track to play now; the one after it becomes `peek()`.
    pub async fn next(&mut self) -> sqlx::Result<Option<Media>> {
        if !self.loaded {
            self.load_history().await?;
        }
        let now = match self.lookahead.take() {
            Some(m) => Some(m),
            None => self.pick().await?,
        };
        self.lookahead = self.pick().await?;
        Ok(now)
    }

    pub fn peek(&self) -> Option<&Media> {
        self.lookahead.as_ref()
    }

    /// Restore shuffle + artist rotation from the DB so a restart doesn't replay recent tracks.
    async fn load_history(&mut self) -> sqlx::Result<()> {
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT h.media_id, m.artist FROM song_history h JOIN media m ON m.id = h.media_id
             WHERE h.station_id = ? AND h.is_live = 0 ORDER BY h.played_at DESC, h.id DESC LIMIT ?",
        )
        .bind(self.station_id)
        .bind(HISTORY as i64)
        .fetch_all(&self.db)
        .await?;
        for (id, artist) in rows.iter().rev() {
            self.played.push_back(*id);
            self.recent.push_back(artist.clone());
            if self.recent.len() > NO_REPEAT_ARTISTS {
                self.recent.pop_front();
            }
        }
        self.loaded = true;
        Ok(())
    }

    fn eligible(&self, p: &Playlist, now: i64) -> bool {
        match p.playlist_type.as_str() {
            "default" => p.weight > 0,
            "once_per_x_songs" => {
                p.play_per > 0 && self.since.get(&p.id).copied().unwrap_or(0) >= p.play_per
            }
            "once_per_x_minutes" => p.play_per > 0 && now - p.played_at >= p.play_per * 60,
            "once_per_hour" => per_hour_due(
                now,
                UTC_OFFSET_MIN.load(Ordering::Relaxed),
                p.play_per_hour_minute,
                p.played_at,
            ),
            _ => false,
        }
    }

    async fn pick(&mut self) -> sqlx::Result<Option<Media>> {
        let pls: Vec<Playlist> = sqlx::query_as(sql!(
            "SELECT {PLAYLIST_COLS} FROM playlists p WHERE station_id = ? AND EXISTS (SELECT 1 FROM playlist_media pm WHERE pm.playlist_id = p.id)"
        ))
        .bind(self.station_id)
        .fetch_all(&self.db)
        .await?;
        let now = unix_now();
        let offset = UTC_OFFSET_MIN.load(Ordering::Relaxed);
        let scheds: Vec<Schedule> = sqlx::query_as(sql!(
            "SELECT {SCHEDULE_COLS} FROM playlist_schedules WHERE playlist_id IN (SELECT id FROM playlists WHERE station_id = ?)"
        ))
        .bind(self.station_id)
        .fetch_all(&self.db)
        .await?;
        // None = no schedule (always on); Some(active?) = scheduled
        let sched_state = |id: i64| {
            let mine: Vec<&Schedule> = scheds.iter().filter(|s| s.playlist_id == id).collect();
            (!mine.is_empty()).then(|| mine.iter().any(|s| s.active(now, offset)))
        };
        // AzuraCast priority: per type, scheduled-and-active playlists first, then unscheduled ones.
        for ty in TYPES_BY_PRIORITY {
            for scheduled in [true, false] {
                let due: Vec<&Playlist> = pls
                    .iter()
                    .filter(|p| p.playlist_type == ty && self.eligible(p, now))
                    .filter(|p| match sched_state(p.id) {
                        Some(active) => scheduled && active,
                        None => !scheduled,
                    })
                    .collect();
                for p in weighted_order(due, ty == "default") {
                    if let Some(m) = self.from_playlist(p).await? {
                        sqlx::query("UPDATE playlists SET played_at = ? WHERE id = ?")
                            .bind(now)
                            .bind(p.id)
                            .execute(&self.db)
                            .await?;
                        self.bump_since(&pls, Some(p.id));
                        self.played_push(&m, ty == "default");
                        return Ok(Some(m));
                    }
                }
            }
        }
        // No (eligible) default playlist: whole catalog, minus tracks that only live in special playlists (jingles etc.)
        let mut all: Vec<Media> = sqlx::query_as(sql!(
            "SELECT {MEDIA_COLS} FROM media WHERE station_id = ? AND id NOT IN
             (SELECT pm.media_id FROM playlist_media pm JOIN playlists p ON p.id = pm.playlist_id WHERE p.playlist_type != 'default')"
        ))
        .bind(self.station_id)
        .fetch_all(&self.db)
        .await?;
        if all.is_empty() {
            all = sqlx::query_as(sql!("SELECT {MEDIA_COLS} FROM media WHERE station_id = ?"))
                .bind(self.station_id)
                .fetch_all(&self.db)
                .await?;
        }
        self.bump_since(&pls, None);
        let m = choose(&all, &self.played, &self.recent, usize::MAX).cloned();
        if let Some(m) = &m {
            self.played_push(m, true);
        }
        Ok(m)
    }

    /// One more song played: every playlist's counter +1 (new ones start at 1), the chosen one back to 0.
    fn bump_since(&mut self, pls: &[Playlist], chosen: Option<i64>) {
        for p in pls {
            *self.since.entry(p.id).or_insert(0) += 1;
        }
        if let Some(id) = chosen {
            self.since.insert(id, 0);
        }
    }

    async fn from_playlist(&mut self, p: &Playlist) -> sqlx::Result<Option<Media>> {
        let cols = MEDIA_COLS
            .split(", ")
            .map(|c| format!("m.{c}"))
            .collect::<Vec<_>>()
            .join(", ");
        if p.play_order == "sequential" {
            // next position after the cursor, wrapping to the start
            let q = sql!(
                "SELECT {cols}, pm.position FROM media m JOIN playlist_media pm ON pm.media_id = m.id
                 WHERE pm.playlist_id = ? ORDER BY (pm.position <= ?), pm.position, m.id LIMIT 1"
            );
            let row: Option<(i64, i64, String, String, String, Option<String>, i64, i64)> =
                sqlx::query_as(q)
                    .bind(p.id)
                    .bind(p.queue_pos)
                    .fetch_optional(&self.db)
                    .await?;
            let Some((id, station_id, file_path, title, artist, album, duration_ms, pos)) = row
            else {
                return Ok(None);
            };
            sqlx::query("UPDATE playlists SET queue_pos = ? WHERE id = ?")
                .bind(pos)
                .bind(p.id)
                .execute(&self.db)
                .await?;
            return Ok(Some(Media {
                id,
                station_id,
                file_path,
                title,
                artist,
                album,
                duration_ms,
            }));
        }
        let cands: Vec<Media> = sqlx::query_as(sql!(
            "SELECT {cols} FROM media m JOIN playlist_media pm ON pm.media_id = m.id WHERE pm.playlist_id = ?"
        ))
        .bind(p.id)
        .fetch_all(&self.db)
        .await?;
        let window = if p.play_order == "random" {
            1
        } else {
            usize::MAX
        };
        let none = VecDeque::new();
        let artists = if p.playlist_type == "default" {
            &self.recent
        } else {
            &none
        };
        Ok(choose(&cands, &self.played, artists, window).cloned())
    }

    fn played_push(&mut self, m: &Media, counts_for_artist_rule: bool) {
        self.played.push_back(m.id);
        if self.played.len() > HISTORY {
            self.played.pop_front();
        }
        if counts_for_artist_rule {
            self.recent.push_back(m.artist.clone());
            if self.recent.len() > NO_REPEAT_ARTISTS {
                self.recent.pop_front();
            }
        }
    }
}

/// Shuffle rotation: never repeat a track until every other candidate played (window = n-1, capped by `window`;
/// window 1 = "random": only no immediate repeat), and avoid the last 3 artists ("Unknown" doesn't count).
/// Rules relax, artist first, only if nothing is left.
fn choose<'a>(
    candidates: &'a [Media],
    played: &VecDeque<i64>,
    artists: &VecDeque<String>,
    window: usize,
) -> Option<&'a Media> {
    let max_tw = candidates
        .len()
        .saturating_sub(1)
        .min(played.len())
        .min(window);
    (0..=max_tw).rev().find_map(|tw| {
        let skip_ids: Vec<i64> = played.iter().rev().take(tw).copied().collect();
        let base: Vec<&Media> = candidates
            .iter()
            .filter(|m| !skip_ids.contains(&m.id))
            .collect();
        let pool = (0..=artists.len()).rev().find_map(|k| {
            let skip: Vec<&String> = artists.iter().rev().take(k).collect();
            let p: Vec<&Media> = base
                .iter()
                .copied()
                .filter(|m| m.artist == "Unknown" || !skip.contains(&&m.artist))
                .collect();
            (!p.is_empty()).then_some(p)
        })?;
        Some(pool[fastrand::usize(..pool.len())])
    })
}

/// Weighted random order without replacement (weight 0 dropped when weights apply; otherwise all equal).
fn weighted_order(mut v: Vec<&Playlist>, use_weight: bool) -> Vec<&Playlist> {
    let w = |p: &Playlist| if use_weight { p.weight.max(0) } else { 1 };
    let mut out = Vec::with_capacity(v.len());
    loop {
        let total: i64 = v.iter().map(|p| w(p)).sum();
        if total == 0 {
            return out;
        }
        let mut r = fastrand::i64(0..total);
        let i = v
            .iter()
            .position(|p| {
                r -= w(p);
                r < 0
            })
            .unwrap();
        out.push(v.remove(i));
    }
}

/// Spawn the AutoDJ thread for a station, fed by its scheduler. Call from inside the tokio runtime.
pub fn start_autodj(db: SqlitePool, mounts: &Mounts, st: &Station) {
    let rt = tokio::runtime::Handle::current();
    let mut sched = Scheduler::new(db.clone(), st.id);
    let mut idle_until = Instant::now();
    let mount = mounts.get_or_create(&st.mountpoint);
    super::history::start_recorder(db.clone(), mount.clone(), st.id);
    let ctl = mount.clone();
    engine::spawn(
        mount,
        st.bitrate as u32,
        st.crossfade_secs as f32,
        move || {
            let forced = ctl.requeue.lock().unwrap().pop_front();
            if let Some(id) = forced
                && let Ok(Some(m)) = rt.block_on(sched.forced(id))
            {
                return Some((
                    PathBuf::from(m.file_path),
                    format!("{} - {}", m.artist, m.title),
                    m.id,
                ));
            }
            if Instant::now() < idle_until {
                return None; // empty catalog: don't hit the DB every 26 ms
            }
            let picked = rt.block_on(sched.next());
            ctl.next_up.store(
                sched.peek().map_or(0, |m| m.id),
                std::sync::atomic::Ordering::SeqCst,
            );
            match picked {
                Ok(Some(m)) => Some((
                    PathBuf::from(m.file_path),
                    format!("{} - {}", m.artist, m.title),
                    m.id,
                )),
                other => {
                    if let Err(e) = other {
                        eprintln!("scheduler: {e}");
                    }
                    idle_until = Instant::now() + Duration::from_secs(2);
                    None
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shuffle_plays_everything_before_repeating() {
        let media: Vec<Media> = (1..=41)
            .map(|id| Media {
                id,
                station_id: 1,
                file_path: String::new(),
                title: String::new(),
                artist: if id <= 8 {
                    "Unknown".into()
                } else {
                    format!("A{}", id % 15)
                },
                album: None,
                duration_ms: 0,
            })
            .collect();
        let (mut played, mut artists) = (VecDeque::new(), VecDeque::new());
        for round in 0..3 {
            let mut seen = std::collections::HashSet::new();
            for _ in 0..media.len() {
                let m = choose(&media, &played, &artists, usize::MAX).unwrap();
                assert!(seen.insert(m.id), "round {round}: {} repeated", m.id);
                played.push_back(m.id);
                artists.push_back(m.artist.clone());
                if artists.len() > NO_REPEAT_ARTISTS {
                    artists.pop_front();
                }
            }
        }
        let one = [media[0].clone()];
        assert_eq!(choose(&one, &played, &artists, usize::MAX).unwrap().id, 1); // single track still plays
    }

    fn pl(id: i64, weight: i64) -> Playlist {
        Playlist {
            id,
            station_id: 1,
            name: String::new(),
            weight,
            playlist_type: "default".into(),
            play_per: 0,
            play_per_hour_minute: 0,
            play_order: "shuffle".into(),
            played_at: 0,
            queue_pos: -1,
            is_jingle: false,
            jingle_interval: None,
        }
    }

    #[test]
    fn weighted_order_skips_zero_and_respects_weights() {
        let pls = [pl(1, 0), pl(2, 90), pl(3, 10)];
        let mut first = [0; 4];
        for _ in 0..10_000 {
            let o = weighted_order(pls.iter().collect(), true);
            assert_eq!(o.len(), 2, "weight 0 dropped");
            first[o[0].id as usize] += 1;
        }
        assert!(first[2] > 8_500 && first[3] > 600, "{first:?}");
        assert_eq!(
            weighted_order(pls.iter().collect(), false).len(),
            3,
            "unweighted keeps all"
        );
    }

    #[test]
    fn once_per_hour_window() {
        let t = |h: i64, m: i64| h * 3600 + m * 60; // UTC
        assert!(per_hour_due(t(10, 0), 0, 0, 0));
        assert!(per_hour_due(t(10, 15), 0, 0, 0));
        assert!(!per_hour_due(t(10, 16), 0, 0, 0));
        assert!(!per_hour_due(t(9, 59), 0, 0, 0));
        assert!(
            per_hour_due(t(10, 2), 0, 55, 0),
            "window wraps the hour: 55..10"
        );
        assert!(!per_hour_due(t(10, 5), 0, 0, t(10, 1)), "played 4 min ago");
        // -180 min offset (Brasília): 13:30 UTC == 10:30 local
        assert!(per_hour_due(t(13, 30), -180, 30, 0));
    }

    #[test]
    fn random_window_only_blocks_immediate_repeat() {
        let media: Vec<Media> = (1..=3)
            .map(|id| Media {
                id,
                station_id: 1,
                file_path: String::new(),
                title: String::new(),
                artist: "Unknown".into(),
                album: None,
                duration_ms: 0,
            })
            .collect();
        let played: VecDeque<i64> = [1, 2].into();
        for _ in 0..200 {
            assert_ne!(choose(&media, &played, &VecDeque::new(), 1).unwrap().id, 2);
        }
    }
}
