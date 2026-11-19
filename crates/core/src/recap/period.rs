//! Recap periods: calendar weeks (Monday to Sunday), months and years of
//! local dates, with ids like `week:2026-11-09` (the period's first day).

use std::fmt;

use chrono::{Datelike, Duration, NaiveDate};

use crate::stats::DateRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PeriodKind {
    Week,
    Month,
    Year,
}

impl PeriodKind {
    pub const ALL: [PeriodKind; 3] = [PeriodKind::Week, PeriodKind::Month, PeriodKind::Year];

    pub fn name(self) -> &'static str {
        match self {
            Self::Week => "week",
            Self::Month => "month",
            Self::Year => "year",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "week" => Some(Self::Week),
            "month" => Some(Self::Month),
            "year" => Some(Self::Year),
            _ => None,
        }
    }

    /// Days with data a finished period needs before it gets a recap.
    pub fn min_active_days(self) -> u32 {
        match self {
            Self::Week => MIN_ACTIVE_DAYS_WEEK,
            Self::Month => MIN_ACTIVE_DAYS_MONTH,
            Self::Year => MIN_ACTIVE_DAYS_YEAR,
        }
    }
}

pub const MIN_ACTIVE_DAYS_WEEK: u32 = 2;
pub const MIN_ACTIVE_DAYS_MONTH: u32 = 7;
pub const MIN_ACTIVE_DAYS_YEAR: u32 = 30;
/// From the 1st of this month the current year gets a recap (1 Jan → today).
pub const YEAR_EARLY_FROM_MONTH: u32 = 12;

/// One week, month or year.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Period {
    pub kind: PeriodKind,
    /// The first day (a Monday, the 1st, or 1 January).
    pub start: NaiveDate,
}

impl Period {
    /// The period of `kind` that contains `date`.
    pub fn containing(kind: PeriodKind, date: NaiveDate) -> Self {
        let start = match kind {
            PeriodKind::Week => {
                date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
            }
            PeriodKind::Month => date.with_day(1).unwrap_or(date),
            PeriodKind::Year => NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap_or(date),
        };
        Self { kind, start }
    }

    /// The last day.
    pub fn end(&self) -> NaiveDate {
        self.next().start.pred_opt().unwrap_or(self.start)
    }

    pub fn range(&self) -> DateRange {
        DateRange {
            from: self.start,
            to: self.end(),
        }
    }

    pub fn next(&self) -> Self {
        let start = match self.kind {
            PeriodKind::Week => self.start + Duration::days(7),
            PeriodKind::Month => {
                let (y, m) = if self.start.month() == 12 {
                    (self.start.year() + 1, 1)
                } else {
                    (self.start.year(), self.start.month() + 1)
                };
                NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(self.start)
            }
            PeriodKind::Year => {
                NaiveDate::from_ymd_opt(self.start.year() + 1, 1, 1).unwrap_or(self.start)
            }
        };
        Self { start, ..*self }
    }

    pub fn previous(&self) -> Self {
        Self::containing(self.kind, self.start.pred_opt().unwrap_or(self.start))
    }

    /// `week:2026-11-09`, `month:2026-11-01`, `year:2026-01-01`.
    pub fn id(&self) -> String {
        format!("{}:{}", self.kind.name(), self.start)
    }

    /// Parse `<kind>:<any date inside it>` (also `month:2026-11`, `year:2026`).
    pub fn parse_id(s: &str) -> Option<Self> {
        let (kind, date) = s.split_once(':')?;
        Some(Self::containing(
            PeriodKind::parse(kind)?,
            parse_loose_date(date)?,
        ))
    }

    /// "9–15 Nov 2026", "27 Oct – 2 Nov 2026", "29 Dec 2025 – 4 Jan 2026",
    /// "November 2026", "2026".
    pub fn label(&self) -> String {
        let (a, b) = (self.start, self.end());
        match self.kind {
            PeriodKind::Week if a.year() != b.year() => {
                format!("{} – {}", a.format("%-d %b %Y"), b.format("%-d %b %Y"))
            }
            PeriodKind::Week if a.month() != b.month() => {
                format!("{} – {}", a.format("%-d %b"), b.format("%-d %b %Y"))
            }
            PeriodKind::Week => format!("{}–{}", a.day(), b.format("%-d %b %Y")),
            PeriodKind::Month => a.format("%B %Y").to_string(),
            PeriodKind::Year => a.year().to_string(),
        }
    }
}

impl fmt::Display for Period {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id())
    }
}

/// `YYYY-MM-DD`, `YYYY-MM` (the 1st) or `YYYY` (1 January).
pub fn parse_loose_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d);
    }
    let parts: Vec<&str> = s.split('-').collect();
    let num = |p: &str| p.parse::<u32>().ok();
    match parts.as_slice() {
        [y, m] if y.len() == 4 => NaiveDate::from_ymd_opt(y.parse().ok()?, num(m)?, 1),
        [y] if y.len() == 4 => NaiveDate::from_ymd_opt(y.parse().ok()?, 1, 1),
        _ => None,
    }
}
