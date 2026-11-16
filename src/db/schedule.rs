//! Playlist time windows (station local time). Pure date math, no date crate.
use serde::Serialize;

#[derive(sqlx::FromRow, Serialize, Clone, Debug)]
pub struct Schedule {
    pub id: i64,
    pub playlist_id: i64,
    pub start_min: i64,
    pub end_min: i64,
    pub days: String,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
}

pub const SCHEDULE_COLS: &str = "id, playlist_id, start_min, end_min, days, start_date, end_date";

/// Local calendar view of a unix timestamp.
struct Local {
    day: i64,    // days since 1970-01-01 (local)
    minute: i64, // minute of day
}

fn local(now: i64, offset_min: i64) -> Local {
    let t = now + offset_min * 60;
    Local {
        day: t.div_euclid(86_400),
        minute: t.rem_euclid(86_400) / 60,
    }
}

/// ISO weekday, Mon=1 .. Sun=7. 1970-01-01 was a Thursday.
fn weekday(day: i64) -> i64 {
    (day + 3).rem_euclid(7) + 1
}

/// Days since epoch -> "YYYY-MM-DD" (Howard Hinnant's civil_from_days).
fn ymd(day: i64) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

impl Schedule {
    /// Is `start_day` (the day the window opens) allowed by weekday + date range?
    fn day_ok(&self, start_day: i64) -> bool {
        let wd = char::from(b'0' + weekday(start_day) as u8);
        let date = ymd(start_day);
        self.days.contains(wd)
            && self
                .start_date
                .as_deref()
                .is_none_or(|s| date.as_str() >= s)
            && self.end_date.as_deref().is_none_or(|e| date.as_str() <= e)
    }

    pub fn active(&self, now: i64, offset_min: i64) -> bool {
        let l = local(now, offset_min);
        let (s, e) = (self.start_min, self.end_min);
        if s == e {
            return self.day_ok(l.day); // full day
        }
        if s < e {
            return (s..e).contains(&l.minute) && self.day_ok(l.day);
        }
        // crosses midnight: evening part belongs to today, early-morning part to yesterday's window
        (l.minute >= s && self.day_ok(l.day)) || (l.minute < e && self.day_ok(l.day - 1))
    }
}

/// "HH:MM" -> minutes after midnight.
pub fn parse_hhmm(v: &str) -> Option<i64> {
    let (h, m) = v.split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

pub fn valid_date(v: &str) -> bool {
    let b = v.as_bytes();
    v.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && v.split('-').all(|p| p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sch(s: &str, e: &str, days: &str) -> Schedule {
        Schedule {
            id: 1,
            playlist_id: 1,
            start_min: parse_hhmm(s).unwrap(),
            end_min: parse_hhmm(e).unwrap(),
            days: days.into(),
            start_date: None,
            end_date: None,
        }
    }
    // 2026-11-16 is a Monday. unix day 20773.
    const MON: i64 = 20773 * 86_400;
    fn at(day_offset: i64, h: i64, m: i64) -> i64 {
        MON + day_offset * 86_400 + h * 3600 + m * 60
    }

    #[test]
    fn calendar_math() {
        assert_eq!(ymd(20773), "2026-11-16");
        assert_eq!(weekday(20773), 1);
        assert_eq!(ymd(0), "1970-01-01");
        assert_eq!(weekday(0), 4);
        assert_eq!(ymd(11016), "2000-02-29");
    }

    #[test]
    fn windows() {
        let day = sch("08:00", "12:00", "1234567");
        assert!(day.active(at(0, 8, 0), 0));
        assert!(day.active(at(0, 11, 59), 0));
        assert!(!day.active(at(0, 12, 0), 0), "end exclusive");
        assert!(!day.active(at(0, 7, 59), 0));

        let night = sch("22:00", "02:00", "6"); // Saturday night into Sunday
        assert!(night.active(at(5, 23, 0), 0), "Sat 23:00");
        assert!(
            night.active(at(6, 1, 30), 0),
            "Sun 01:30 belongs to Sat window"
        );
        assert!(!night.active(at(6, 23, 0), 0), "Sun 23:00 not allowed");
        assert!(
            !night.active(at(0, 1, 0), 0),
            "Mon 01:00 belongs to Sun window"
        );

        // Brasília offset: 11:00 UTC == 08:00 local
        assert!(day.active(at(0, 11, 0), -180));
        assert!(!day.active(at(0, 8, 0), -180));

        let mut dated = sch("00:00", "00:00", "1234567");
        dated.start_date = Some("2026-11-17".into());
        assert!(!dated.active(at(0, 10, 0), 0));
        assert!(dated.active(at(1, 10, 0), 0));
    }

    #[test]
    fn parsing() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("7"), None);
        assert!(valid_date("2026-11-16"));
        assert!(!valid_date("2026-1-16"));
    }
}
