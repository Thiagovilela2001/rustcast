//! Background loudness (EBU R128) + auto-cue analysis. One file at a time, streamed in 100 ms blocks
//! (constant RAM), paced so it never competes with the real-time AutoDJ thread on a small VM.
use super::deck::{Deck, RATE};
use ebur128::{EbuR128, Mode};
use sqlx::SqlitePool;
use std::path::Path;
use std::time::Duration;

const BLOCK_FRAMES: usize = (RATE / 10) as usize; // 100 ms
const SILENCE_DB: f64 = -45.0;
const MIN_GAIN_DB: f64 = -12.0;
const MAX_GAIN_DB: f64 = 9.0;
const PEAK_CEILING_DB: f64 = -1.0;
/// Sleep per 100 ms block: caps analysis near 50x real time and yields the CPU.
const PACE: Duration = Duration::from_millis(2);

#[derive(Debug, PartialEq)]
pub struct Analysis {
    pub lufs: Option<f64>,
    pub peak_db: f64,
    pub cue_in_ms: i64,
    pub cue_out_ms: Option<i64>,
}

fn db(v: f64) -> f64 {
    if v > 0.0 { 20.0 * v.log10() } else { f64::NEG_INFINITY }
}

/// Feed interleaved stereo blocks; returns loudness, peak and the first/last non-silent block.
pub fn analyze_blocks(mut next_block: impl FnMut(&mut [f32]) -> usize) -> Result<Analysis, ebur128::Error> {
    let mut meter = EbuR128::new(2, RATE, Mode::I | Mode::SAMPLE_PEAK)?;
    let mut buf = vec![0f32; BLOCK_FRAMES * 2];
    let (mut idx, mut first, mut last, mut total_frames) = (0i64, None, None, 0i64);
    loop {
        let n = next_block(&mut buf);
        if n == 0 {
            break;
        }
        meter.add_frames_f32(&buf[..n * 2])?;
        let peak = buf[..n * 2].iter().fold(0f32, |m, s| m.max(s.abs())) as f64;
        if db(peak) > SILENCE_DB {
            first.get_or_insert(idx);
            last = Some(idx);
        }
        total_frames += n as i64;
        idx += 1;
        if n < BLOCK_FRAMES {
            break;
        }
    }
    let lufs = meter.loudness_global().ok().filter(|l| l.is_finite());
    let peak = meter.sample_peak(0)?.max(meter.sample_peak(1)?);
    let ms = |block: i64| block * 100;
    let total_ms = total_frames * 1000 / RATE as i64;
    let (cue_in_ms, cue_out_ms) = match (first, last) {
        // trim only real silence, and never to a stub
        (Some(f), Some(l)) if ms(l + 1) - ms(f) >= 5_000 => {
            let cin = if f >= 3 { ms(f) } else { 0 }; // ignore < 300 ms of lead-in
            let cout = (ms(l + 1) + 200).min(total_ms); // keep a 200 ms tail
            (cin, (total_ms - cout > 300).then_some(cout))
        }
        _ => (0, None),
    };
    Ok(Analysis { lufs, peak_db: db(peak), cue_in_ms, cue_out_ms })
}

pub fn analyze_file(path: &Path) -> Result<Analysis, Box<dyn std::error::Error>> {
    let mut deck = Deck::open(path)?;
    Ok(analyze_blocks(|buf| {
        std::thread::sleep(PACE);
        deck.read(buf)
    })?)
}

/// Gain to reach `target_lufs`, clamped, never pushing the peak above -1 dBFS.
pub fn gain_for(a: &Analysis, target_lufs: f64) -> f64 {
    let Some(l) = a.lufs else { return 0.0 };
    let headroom = PEAK_CEILING_DB - a.peak_db;
    (target_lufs - l).clamp(MIN_GAIN_DB, MAX_GAIN_DB).min(headroom.max(MIN_GAIN_DB))
}

/// Analyze pending media forever (new uploads included). Call from inside the tokio runtime.
pub fn start_worker(db: SqlitePool, target_lufs: f64) {
    tokio::spawn(async move {
        loop {
            let row: Option<(i64, String)> = sqlx::query_as("SELECT id, file_path FROM media WHERE analyzed = 0 ORDER BY id LIMIT 1")
                .fetch_optional(&db)
                .await
                .ok()
                .flatten();
            let Some((id, path)) = row else {
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            };
            let res = tokio::task::spawn_blocking(move || analyze_file(Path::new(&path)).map_err(|e| e.to_string())).await;
            let q = match res {
                Ok(Ok(a)) => sqlx::query(
                    "UPDATE media SET analyzed = 1, loudness_lufs = ?, peak_db = ?, gain_db = ?, cue_in_ms = ?, cue_out_ms = ? WHERE id = ?",
                )
                .bind(a.lufs)
                .bind(a.peak_db.max(-200.0))
                .bind(gain_for(&a, target_lufs))
                .bind(a.cue_in_ms)
                .bind(a.cue_out_ms)
                .bind(id),
                other => {
                    eprintln!("analyze: media {id} failed: {other:?}");
                    sqlx::query("UPDATE media SET analyzed = 2 WHERE id = ?").bind(id)
                }
            };
            if let Err(e) = q.execute(&db).await {
                eprintln!("analyze: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1 s silence, `secs` of a sine at `amp`, 2 s silence.
    fn signal(amp: f32, secs: usize) -> Vec<f32> {
        let r = RATE as usize;
        let mut v = vec![0f32; r * 2];
        for i in 0..r * secs {
            let s = amp * (i as f32 * 2.0 * std::f32::consts::PI * 1000.0 / RATE as f32).sin();
            v.extend([s, s]);
        }
        v.extend(vec![0f32; r * 2 * 2]);
        v
    }

    fn run(sig: &[f32]) -> Analysis {
        let mut pos = 0;
        analyze_blocks(|buf| {
            let n = (sig.len() - pos).min(buf.len());
            buf[..n].copy_from_slice(&sig[pos..pos + n]);
            pos += n;
            n / 2
        })
        .unwrap()
    }

    #[test]
    fn loud_vs_quiet_and_cues() {
        let loud = run(&signal(0.5, 10));
        let quiet = run(&signal(0.05, 10));
        let (ll, ql) = (loud.lufs.unwrap(), quiet.lufs.unwrap());
        assert!((ll - ql - 20.0).abs() < 0.5, "10x amplitude = +20 LU: {ll} vs {ql}");
        assert_eq!(loud.cue_in_ms, 1000, "1 s lead-in trimmed");
        assert_eq!(loud.cue_out_ms, Some(11_200), "music ends at 11 s, +200 ms tail");
        // quiet track gets boosted, loud one cut, both land near the target where headroom allows
        let (gl, gq) = (gain_for(&loud, -16.0), gain_for(&quiet, -16.0));
        assert!(gq > 0.0 && gl < gq, "{gl} {gq}");
        assert!(quiet.peak_db + gq <= PEAK_CEILING_DB + 1e-6, "never clip");
    }

    #[test]
    fn silence_and_short_files_are_left_alone() {
        let silent = run(&vec![0f32; RATE as usize * 2 * 3]);
        assert_eq!((silent.lufs, silent.cue_in_ms, silent.cue_out_ms), (None, 0, None));
        assert_eq!(gain_for(&silent, -16.0), 0.0);
        let stub = run(&signal(0.5, 2)); // 2 s of audio: too short to trim
        assert_eq!((stub.cue_in_ms, stub.cue_out_ms), (0, None));
    }
}
