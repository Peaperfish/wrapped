//! Time handling.
//!
//! Every stored timestamp is UTC Unix milliseconds plus the local UTC offset
//! (minutes east of UTC) at that moment. Durations are always computed from
//! wall-clock UTC differences, never by counting timer ticks (timers pause
//! while the machine sleeps).

use std::cmp::Ordering;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Local, NaiveDate, Offset, TimeZone, Utc};

pub const MS_PER_SEC: i64 = 1_000;
pub const MS_PER_MIN: i64 = 60 * MS_PER_SEC;
pub const MS_PER_HOUR: i64 = 60 * MS_PER_MIN;
pub const MS_PER_DAY: i64 = 24 * MS_PER_HOUR;

/// A moment in time: UTC milliseconds plus the local UTC offset in force then.
///
/// Equality and ordering use only `utc_ms`: two timestamps describing the same
/// instant in different zones are the same moment.
#[derive(Debug, Clone, Copy)]
pub struct Timestamp {
    pub utc_ms: i64,
    /// Minutes east of UTC (e.g. +480 for UTC+8, -300 for UTC-5).
    pub offset_min: i32,
}

impl Timestamp {
    pub const fn new(utc_ms: i64, offset_min: i32) -> Self {
        Self { utc_ms, offset_min }
    }

    /// Milliseconds since the Unix epoch in *local* wall-clock terms.
    pub fn local_ms(&self) -> i64 {
        self.utc_ms + i64::from(self.offset_min) * MS_PER_MIN
    }

    /// The local calendar date of this moment.
    pub fn local_date(&self) -> NaiveDate {
        date_from_local_ms(self.local_ms())
    }

    /// Same offset, moved by `ms`.
    pub fn plus_ms(&self, ms: i64) -> Self {
        Self::new(self.utc_ms + ms, self.offset_min)
    }

    /// Same instant expressed with another offset.
    pub fn with_utc_ms(&self, utc_ms: i64) -> Self {
        Self::new(utc_ms, self.offset_min)
    }

    /// RFC 3339 rendering in the stored local offset, e.g. `2026-11-16T14:20:47+08:00`.
    pub fn to_rfc3339(&self) -> String {
        let off = chrono::FixedOffset::east_opt(self.offset_min * 60)
            .unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("zero offset"));
        match off.timestamp_millis_opt(self.utc_ms).single() {
            Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
            None => format!("{}ms", self.utc_ms),
        }
    }
}

impl PartialEq for Timestamp {
    fn eq(&self, other: &Self) -> bool {
        self.utc_ms == other.utc_ms
    }
}
impl Eq for Timestamp {}
impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.utc_ms.cmp(&other.utc_ms)
    }
}

/// Calendar date for a local-milliseconds value.
pub fn date_from_local_ms(local_ms: i64) -> NaiveDate {
    let days = local_ms.div_euclid(MS_PER_DAY);
    epoch_date()
        .checked_add_signed(chrono::Duration::days(days))
        .unwrap_or(NaiveDate::MIN)
}

/// Local-milliseconds value of 00:00 on `date`.
pub fn local_midnight_ms(date: NaiveDate) -> i64 {
    (date - epoch_date()).num_days() * MS_PER_DAY
}

fn epoch_date() -> NaiveDate {
    NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date")
}

/// Source of "now". Collectors and the engine only ever read time through this,
/// so tests can drive everything with a [`ManualClock`].
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// The real wall clock with the system's current local offset.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        let utc: DateTime<Utc> = Utc::now();
        let offset_secs = Local
            .offset_from_utc_datetime(&utc.naive_utc())
            .fix()
            .local_minus_utc();
        Timestamp::new(utc.timestamp_millis(), offset_secs / 60)
    }
}

/// A clock moved by hand. Used by tests; cheap to clone (shared state).
#[derive(Debug, Clone)]
pub struct ManualClock(Arc<Mutex<Timestamp>>);

impl ManualClock {
    pub fn new(start: Timestamp) -> Self {
        Self(Arc::new(Mutex::new(start)))
    }
    /// Start at a local date/time given as `(y, m, d, hh, mm, ss)` in `offset_min`.
    pub fn at_local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32, offset_min: i32) -> Self {
        let date = NaiveDate::from_ymd_opt(y, mo, d).expect("valid date");
        let local = local_midnight_ms(date)
            + i64::from(h) * MS_PER_HOUR
            + i64::from(mi) * MS_PER_MIN
            + i64::from(s) * MS_PER_SEC;
        Self::new(Timestamp::new(
            local - i64::from(offset_min) * MS_PER_MIN,
            offset_min,
        ))
    }
    pub fn advance_secs(&self, secs: i64) {
        self.advance_ms(secs * MS_PER_SEC);
    }
    pub fn advance_ms(&self, ms: i64) {
        let mut t = self.0.lock().expect("clock lock");
        t.utc_ms += ms;
    }
    pub fn set_offset_min(&self, offset_min: i32) {
        self.0.lock().expect("clock lock").offset_min = offset_min;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        *self.0.lock().expect("clock lock")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_date_uses_offset() {
        // 2026-11-16 23:30 at UTC+8 is 15:30Z the same day.
        let c = ManualClock::at_local(2026, 11, 16, 23, 30, 0, 480);
        let t = c.now();
        assert_eq!(t.local_date().to_string(), "2026-11-16");
        // In UTC-5 the same instant is 10:30 on the 16th.
        let t2 = Timestamp::new(t.utc_ms, -300);
        assert_eq!(t2.local_date().to_string(), "2026-11-16");
        // One hour later it is the 17th in UTC+8.
        assert_eq!(
            t.plus_ms(MS_PER_HOUR).local_date().to_string(),
            "2026-11-17"
        );
    }

    #[test]
    fn negative_local_ms_dates() {
        assert_eq!(date_from_local_ms(-1).to_string(), "1969-12-31");
        assert_eq!(date_from_local_ms(0).to_string(), "1970-01-01");
    }

    #[test]
    fn rfc3339_in_stored_offset() {
        let c = ManualClock::at_local(2026, 11, 16, 14, 20, 47, 480);
        assert_eq!(c.now().to_rfc3339(), "2026-11-16T14:20:47+08:00");
    }

    #[test]
    fn system_clock_offset_is_whole_minutes_in_range() {
        let t = SystemClock.now();
        assert!((-14 * 60..=14 * 60).contains(&t.offset_min));
    }
}
