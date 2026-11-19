//! Recaps (DESIGN.md §11): weekly, monthly and yearly story slides built from
//! the stats engine. Pure reads; no UI, no platform code.
//!
//! * Periods are local calendar dates (Monday–Sunday weeks, calendar months
//!   and years), the same days the stats engine uses: every second, item and
//!   byte already sits on the local date its collector's rollup gave it (UTC
//!   plus the stored offset), so a trip mid-week moves nothing across weeks.
//! * A finished period gets a recap when it has enough *active days* (days
//!   with any stored metric of a collector that is on): week ≥ 2, month ≥ 7,
//!   year ≥ 30. From 1 December the current year also gets one (1 January →
//!   today), which keeps updating until 31 December.
//! * One [`Stats::summary_vs`] call per recap: the slides are derived from it
//!   (no SQL of their own). Changes compare with the previous calendar period
//!   (for the year in progress: the same days of the year before) and are
//!   `None` when that period is too thin, the dashboard's rule: fewer than
//!   half as many days with data as this one.
//! * Slides of collectors that are off, or without data, are left out. Only
//!   what the dashboard already shows: app ids, repository and AI project
//!   folder names, camera and lens models. No window titles, files or paths.

mod period;

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde_json::{Value, json};

pub use period::{
    MIN_ACTIVE_DAYS_MONTH, MIN_ACTIVE_DAYS_WEEK, MIN_ACTIVE_DAYS_YEAR, Period, PeriodKind,
    YEAR_EARLY_FROM_MONTH, parse_loose_date,
};

use crate::stats::{DateRange, LatestNight, LongestSession, Stats, Streak, Summary, pct_change};
use crate::store::Result as StoreResult;

/// Keys per keyed metric a recap reads (top apps shows five).
pub const RECAP_TOP_N: usize = 5;
/// Photo metric key for a missing camera, lens or focal length.
const UNKNOWN_KEY: &str = "(unknown)";

// ---------- time-of-day label ----------

/// Night: 22:00–05:00 local (hours 22, 23, 0..=4).
pub const NIGHT_HOURS: [usize; 7] = [22, 23, 0, 1, 2, 3, 4];
/// Morning: 05:00–12:00 local (hours 5..=11).
pub const MORNING_HOURS: [usize; 7] = [5, 6, 7, 8, 9, 10, 11];
/// At least this share of active time at night: night owl.
pub const NIGHT_OWL_SHARE: f64 = 0.25;
/// Otherwise, at least this share in the morning: early bird.
pub const EARLY_BIRD_SHARE: f64 = 0.40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayLabel {
    EarlyBird,
    Daytime,
    NightOwl,
}

impl DayLabel {
    pub fn name(self) -> &'static str {
        match self {
            Self::EarlyBird => "early_bird",
            Self::Daytime => "daytime",
            Self::NightOwl => "night_owl",
        }
    }
}

/// Shares of `hours` in the morning and at night, and the label they give
/// (night owl first: ≥ 25 % at night; else early bird: ≥ 40 % in the
/// morning; else daytime). All zeros: daytime.
pub fn day_label(hours: &[f64; 24]) -> (DayLabel, f64, f64) {
    let total: f64 = hours.iter().sum();
    if total <= 0.0 {
        return (DayLabel::Daytime, 0.0, 0.0);
    }
    let share = |hs: &[usize]| hs.iter().map(|h| hours[*h]).sum::<f64>() / total;
    let (morning, night) = (share(&MORNING_HOURS), share(&NIGHT_HOURS));
    let label = if night >= NIGHT_OWL_SHARE {
        DayLabel::NightOwl
    } else if morning >= EARLY_BIRD_SHARE {
        DayLabel::EarlyBird
    } else {
        DayLabel::Daytime
    };
    (label, morning, night)
}

// ---------- availability ----------

/// Whether `period` gets a recap on `today` with `active_days` days of data:
/// it has ended (or it is the current year from 1 December) and has enough days.
pub fn is_available(period: Period, today: NaiveDate, active_days: u32) -> bool {
    shown_on(period, today) && active_days >= period.kind.min_active_days()
}

/// Ended before `today`, or the current year from 1 December.
fn shown_on(period: Period, today: NaiveDate) -> bool {
    period.end() < today || year_in_progress(period, today)
}

fn year_in_progress(period: Period, today: NaiveDate) -> bool {
    period.kind == PeriodKind::Year
        && period.start.year() == today.year()
        && today.month() >= YEAR_EARLY_FROM_MONTH
}

/// The days a recap of `period` covers on `today`: the whole period, or up to
/// `today` while it runs. `None` if it has not started.
pub fn covered(period: Period, today: NaiveDate) -> Option<DateRange> {
    let r = period.range();
    (r.from <= today).then(|| DateRange {
        from: r.from,
        to: r.to.min(today),
    })
}

/// What a recap of `period` is compared with: the previous period, or for a
/// period still running the same number of days at the start of the previous one.
pub fn comparison_range(period: Period, covered: DateRange) -> DateRange {
    let prev = period.previous().range();
    DateRange {
        from: prev.from,
        to: (prev.from + Duration::days(covered.days() - 1)).min(prev.to),
    }
}

/// One entry of the recap list.
#[derive(Debug, Clone, PartialEq)]
pub struct RecapEntry {
    pub period: Period,
    pub range: DateRange,
    pub in_progress: bool,
    pub active_days: u32,
    /// `None` when app usage is off.
    pub screen_secs: Option<f64>,
}

/// Every recap available on `today`: years, then months, then weeks, newest
/// first in each group. Two queries over all stored days.
pub fn list(stats: &Stats<'_>, today: NaiveDate) -> StoreResult<Vec<RecapEntry>> {
    let Some(first) = stats.earliest_date()? else {
        return Ok(Vec::new());
    };
    if first > today {
        return Ok(Vec::new());
    }
    let all = DateRange {
        from: first,
        to: today,
    };
    let days = stats.data_dates(all)?;
    let screen: Option<BTreeMap<NaiveDate, f64>> = if stats.collector_visible("app_usage") {
        Some(
            stats
                .series("app_usage", "screen_time_secs", None, all)?
                .into_iter()
                .filter(|(_, v)| *v > 0.0)
                .collect(),
        )
    } else {
        None
    };
    let mut out = Vec::new();
    for kind in [PeriodKind::Year, PeriodKind::Month, PeriodKind::Week] {
        let mut p = Period::containing(kind, today);
        let stop = Period::containing(kind, first);
        loop {
            if let Some(r) = covered(p, today) {
                let active = days.range(r.from..=r.to).count() as u32;
                if is_available(p, today, active) {
                    out.push(RecapEntry {
                        period: p,
                        range: r,
                        in_progress: p.end() >= today,
                        active_days: active,
                        screen_secs: screen
                            .as_ref()
                            .map(|s| s.range(r.from..=r.to).fold(0.0, |a, (_, v)| a + v)),
                    });
                }
            }
            if p <= stop {
                break;
            }
            p = p.previous();
        }
    }
    Ok(out)
}

// ---------- slides ----------

/// A key and its value (an app, repo, camera, tool ...).
#[derive(Debug, Clone, PartialEq)]
pub struct Keyed {
    pub key: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppShare {
    pub key: String,
    pub secs: f64,
    /// Of the period's screen time, 0..1.
    pub share: f64,
}

/// One headline number of the summary slide.
#[derive(Debug, Clone, PartialEq)]
pub struct Headline {
    /// `screen_time`, `top_app`, `commits`, `photos`, `ai_tokens`, `streak`,
    /// `disk_written` or `active_days`.
    pub id: &'static str,
    pub value: f64,
    /// The app id for `top_app`.
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Slide {
    Intro {
        label: String,
        days: i64,
        active_days: u32,
        screen_secs: Option<f64>,
    },
    ScreenTime {
        total_secs: f64,
        /// Per day with screen time.
        daily_avg_secs: f64,
        screen_days: u32,
        change_pct: Option<f64>,
        /// Monday = 0; the weekday with the highest average, and that average.
        busiest_weekday: Option<(u32, f64)>,
    },
    TopApps {
        total_secs: f64,
        apps: Vec<AppShare>,
    },
    TimeOfDay {
        hours: [f64; 24],
        peak_hour: u32,
        label: DayLabel,
        morning_share: f64,
        night_share: f64,
    },
    BusiestDay {
        date: NaiveDate,
        secs: f64,
    },
    Records {
        longest_session: Option<LongestSession>,
        latest_night: Option<LatestNight>,
        streak: Option<Streak>,
    },
    Git {
        commits: f64,
        change_pct: Option<f64>,
        top_repo: Option<Keyed>,
        busiest_hour: Option<u32>,
        hours: [f64; 24],
    },
    Photos {
        count: f64,
        change_pct: Option<f64>,
        top_camera: Option<Keyed>,
        top_lens: Option<Keyed>,
        /// Key: whole millimetres.
        top_focal: Option<Keyed>,
    },
    Ai {
        tokens: f64,
        input: f64,
        output: f64,
        change_pct: Option<f64>,
        top_tool: Option<Keyed>,
        top_model: Option<Keyed>,
        top_project: Option<Keyed>,
    },
    Disk {
        bytes_written: f64,
        change_pct: Option<f64>,
    },
    Summary {
        items: Vec<Headline>,
    },
}

impl Slide {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Intro { .. } => "intro",
            Self::ScreenTime { .. } => "screen_time",
            Self::TopApps { .. } => "top_apps",
            Self::TimeOfDay { .. } => "time_of_day",
            Self::BusiestDay { .. } => "busiest_day",
            Self::Records { .. } => "records",
            Self::Git { .. } => "git",
            Self::Photos { .. } => "photos",
            Self::Ai { .. } => "ai",
            Self::Disk { .. } => "disk",
            Self::Summary { .. } => "summary",
        }
    }
}

/// Every slide kind, in recap order.
pub const SLIDE_KINDS: [&str; 11] = [
    "intro",
    "screen_time",
    "top_apps",
    "time_of_day",
    "busiest_day",
    "records",
    "git",
    "photos",
    "ai",
    "disk",
    "summary",
];

/// Max headline numbers on the summary slide.
pub const SUMMARY_ITEMS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct Recap {
    pub period: Period,
    /// The days covered (up to today while the period runs).
    pub range: DateRange,
    pub in_progress: bool,
    /// What changes compare with.
    pub previous: DateRange,
    /// False when the previous period is too thin: every change is `None`.
    pub comparable: bool,
    pub active_days: u32,
    /// Listed (enough days, period over or the December year rule).
    pub available: bool,
    pub slides: Vec<Slide>,
}

/// Lookups into one summary.
struct View<'a> {
    sum: &'a Summary,
    comparable: bool,
}

impl View<'_> {
    fn total(&self, c: &str, m: &str) -> f64 {
        self.sum
            .totals
            .iter()
            .find(|t| t.collector == c && t.metric == m && t.key.is_empty())
            .map_or(0.0, |t| t.value)
    }
    fn keyed_total(&self, c: &str, m: &str, key: &str) -> f64 {
        self.sum
            .totals
            .iter()
            .find(|t| t.collector == c && t.metric == m && t.key == key)
            .map_or(0.0, |t| t.value)
    }
    fn change(&self, c: &str, m: &str) -> Option<f64> {
        if !self.comparable {
            return None;
        }
        self.sum
            .comparisons
            .iter()
            .find(|x| x.collector == c && x.metric == m)
            .and_then(|x| pct_change(x.current, x.previous))
    }
    fn top(&self, c: &str, m: &str) -> &[(String, f64)] {
        self.sum
            .top
            .iter()
            .find(|(r, _)| r.collector == c && r.def.id == m)
            .map_or(&[], |(_, v)| v.as_slice())
    }
    /// Best key that is not "(unknown)".
    fn first_known(&self, c: &str, m: &str) -> Option<Keyed> {
        self.top(c, m)
            .iter()
            .find(|(k, v)| k != UNKNOWN_KEY && !k.is_empty() && *v > 0.0)
            .map(|(k, v)| Keyed {
                key: k.clone(),
                value: *v,
            })
    }
    fn hours(&self, c: &str, m: &str) -> [f64; 24] {
        self.sum
            .time_of_day
            .iter()
            .find(|p| p.collector == c && p.metric == m)
            .map_or([0.0; 24], |p| p.hours)
    }
    fn series(&self, c: &str, m: &str) -> &[(NaiveDate, f64)] {
        self.sum
            .series
            .iter()
            .find(|(r, _)| r.collector == c && r.def.id == m)
            .map_or(&[], |(_, v)| v.as_slice())
    }
}

/// Hour with the highest value (the earliest if tied); `None` if all zero.
fn peak(hours: &[f64; 24]) -> Option<u32> {
    let mut best: Option<(usize, f64)> = None;
    for (h, v) in hours.iter().enumerate() {
        if *v > 0.0 && best.is_none_or(|(_, b)| *v > b) {
            best = Some((h, *v));
        }
    }
    best.map(|(h, _)| h as u32)
}

/// The previous period is too thin for a change: fewer than half as many
/// days with data as this one (the dashboard's "—" rule).
pub fn too_thin(data_days: u32, previous_data_days: u32) -> bool {
    previous_data_days * 2 < data_days
}

/// The recap of `period` as of `today` (`None` if it has not started).
pub fn build(stats: &Stats<'_>, period: Period, today: NaiveDate) -> StoreResult<Option<Recap>> {
    let Some(range) = covered(period, today) else {
        return Ok(None);
    };
    let previous = comparison_range(period, range);
    let sum = stats.summary_vs(range, Some(previous), RECAP_TOP_N)?;
    let comparable = !too_thin(sum.data_days, sum.previous_data_days);
    let v = View {
        sum: &sum,
        comparable,
    };
    let active_days = sum.data_days;
    let apps_on = stats.collector_visible("app_usage");
    let mut slides = Vec::new();

    let screen = v.total("app_usage", "screen_time_secs");
    slides.push(Slide::Intro {
        label: period.label(),
        days: range.days(),
        active_days,
        screen_secs: apps_on.then_some(screen),
    });

    if apps_on && screen > 0.0 {
        let days = v.series("app_usage", "screen_time_secs");
        let screen_days = days.iter().filter(|(_, x)| *x > 0.0).count() as u32;
        let mut by_wd = [(0.0f64, 0u32); 7];
        for (d, x) in days {
            let w = d.weekday().num_days_from_monday() as usize;
            by_wd[w].0 += x;
            by_wd[w].1 += 1;
        }
        let busiest_weekday = by_wd
            .iter()
            .enumerate()
            .filter(|(_, (s, n))| *n > 0 && *s > 0.0)
            .map(|(w, (s, n))| (w as u32, s / f64::from(*n)))
            .fold(None::<(u32, f64)>, |best, cur| match best {
                Some(b) if b.1 >= cur.1 => Some(b),
                _ => Some(cur),
            });
        slides.push(Slide::ScreenTime {
            total_secs: screen,
            daily_avg_secs: screen / f64::from(screen_days.max(1)),
            screen_days,
            change_pct: v.change("app_usage", "screen_time_secs"),
            busiest_weekday,
        });
        let apps: Vec<AppShare> = v
            .top("app_usage", "app_secs")
            .iter()
            .filter(|(_, s)| *s > 0.0)
            .take(5)
            .map(|(k, s)| AppShare {
                key: k.clone(),
                secs: *s,
                share: s / screen,
            })
            .collect();
        if !apps.is_empty() {
            slides.push(Slide::TopApps {
                total_secs: screen,
                apps,
            });
        }
        let hours = v.hours("app_usage", "active_secs_by_hour");
        if let Some(peak_hour) = peak(&hours) {
            let (label, morning_share, night_share) = day_label(&hours);
            slides.push(Slide::TimeOfDay {
                hours,
                peak_hour,
                label,
                morning_share,
                night_share,
            });
        }
        if let Some((date, secs)) =
            days.iter()
                .fold(None::<(NaiveDate, f64)>, |b, (d, x)| match b {
                    Some(b) if b.1 >= *x => Some(b),
                    _ if *x > 0.0 => Some((*d, *x)),
                    b => b,
                })
        {
            slides.push(Slide::BusiestDay { date, secs });
        }
        let rec = &sum.records;
        if rec.longest_session.is_some()
            || rec.latest_night.is_some()
            || rec.active_streak.is_some()
        {
            slides.push(Slide::Records {
                longest_session: rec.longest_session.clone(),
                latest_night: rec.latest_night.clone(),
                streak: rec.active_streak,
            });
        }
    }

    let commits = v.total("git", "commits");
    if stats.collector_visible("git") && commits > 0.0 {
        let hours = v.hours("git", "commits_by_hour");
        slides.push(Slide::Git {
            commits,
            change_pct: v.change("git", "commits"),
            top_repo: v.first_known("git", "commits_by_repo"),
            busiest_hour: peak(&hours),
            hours,
        });
    }

    let shots = v.total("photos", "shots");
    if stats.collector_visible("photos") && shots > 0.0 {
        slides.push(Slide::Photos {
            count: shots,
            change_pct: v.change("photos", "shots"),
            top_camera: v.first_known("photos", "shots_by_camera"),
            top_lens: v.first_known("photos", "shots_by_lens"),
            top_focal: v.first_known("photos", "shots_by_focal_length"),
        });
    }

    let tokens = v.total("ai_usage", "tokens");
    if stats.collector_visible("ai_usage") && tokens > 0.0 {
        slides.push(Slide::Ai {
            tokens,
            input: v.keyed_total("ai_usage", "tokens_by_kind", "input"),
            output: v.keyed_total("ai_usage", "tokens_by_kind", "output"),
            change_pct: v.change("ai_usage", "tokens"),
            top_tool: v.first_known("ai_usage", "tokens_by_tool"),
            top_model: v.first_known("ai_usage", "tokens_by_model"),
            top_project: v.first_known("ai_usage", "tokens_by_project"),
        });
    }

    let written = v.total("disk_memory", "bytes_written");
    if stats.collector_visible("disk_memory") && written > 0.0 {
        slides.push(Slide::Disk {
            bytes_written: written,
            change_pct: v.change("disk_memory", "bytes_written"),
        });
    }

    let items = headlines(&slides, active_days);
    slides.push(Slide::Summary { items });

    Ok(Some(Recap {
        period,
        range,
        in_progress: period.end() >= today,
        previous,
        comparable,
        active_days,
        available: is_available(period, today, active_days),
        slides,
    }))
}

/// Up to [`SUMMARY_ITEMS`] headline numbers from the slides, in a fixed
/// priority: screen time, top app, commits, photos, AI tokens, longest
/// streak, disk written, active days.
fn headlines(slides: &[Slide], active_days: u32) -> Vec<Headline> {
    let h = |id, value| Headline {
        id,
        value,
        text: None,
    };
    let mut out = Vec::new();
    for s in slides {
        match s {
            Slide::ScreenTime { total_secs, .. } => out.push((0, h("screen_time", *total_secs))),
            Slide::TopApps { apps, .. } => {
                if let Some(a) = apps.first() {
                    out.push((
                        1,
                        Headline {
                            id: "top_app",
                            value: a.secs,
                            text: Some(a.key.clone()),
                        },
                    ));
                }
            }
            Slide::Git { commits, .. } => out.push((2, h("commits", *commits))),
            Slide::Photos { count, .. } => out.push((3, h("photos", *count))),
            Slide::Ai { tokens, .. } => out.push((4, h("ai_tokens", *tokens))),
            Slide::Records {
                streak: Some(st), ..
            } if st.days > 1 => out.push((5, h("streak", st.days as f64))),
            Slide::Disk { bytes_written, .. } => out.push((6, h("disk_written", *bytes_written))),
            _ => {}
        }
    }
    out.push((7, h("active_days", f64::from(active_days))));
    out.sort_by_key(|(p, _)| *p);
    out.into_iter()
        .map(|(_, x)| x)
        .take(SUMMARY_ITEMS)
        .collect()
}

// ---------- notifications ----------

/// Local hour from which a new recap may be announced.
pub const NOTIFY_HOUR: u32 = 9;

/// When `period`'s recap may be announced: 09:00 local the day after it ends;
/// a year at 09:00 on 1 December (its in-progress recap appears then).
pub fn ready_at(period: Period) -> NaiveDateTime {
    let day = match period.kind {
        PeriodKind::Year => NaiveDate::from_ymd_opt(period.start.year(), YEAR_EARLY_FROM_MONTH, 1)
            .unwrap_or(period.start),
        _ => period.next().start,
    };
    day.and_time(NaiveTime::from_hms_opt(NOTIFY_HOUR, 0, 0).unwrap_or_default())
}

/// The newest period of `kind` that may be announced at local time `now`.
pub fn latest_ready(kind: PeriodKind, now: NaiveDateTime) -> Period {
    let mut p = Period::containing(kind, now.date());
    while ready_at(p) > now {
        p = p.previous();
    }
    p
}

/// What to store as "last announced" when notifications for `kind` are first
/// turned on: the period before the newest ready one, so only that newest
/// one can still be announced, never older ones.
pub fn notify_baseline(kind: PeriodKind, now: NaiveDateTime) -> Period {
    latest_ready(kind, now).previous()
}

/// The period to announce now, if any: only ever the newest ready period,
/// and only when it is newer than `last`, has a recap (`available`) and
/// `now` is past its [`ready_at`]. After a long sleep this is one
/// notification for the newest period, never a burst of old ones.
pub fn due(
    kind: PeriodKind,
    now: NaiveDateTime,
    last: Option<Period>,
    available: &[RecapEntry],
) -> Option<Period> {
    let p = latest_ready(kind, now);
    if last.is_some_and(|l| l >= p) {
        return None;
    }
    available.iter().any(|e| e.period == p).then_some(p)
}

/// Hours with at most one decimal under 10 h ("4.5 h", "32 h").
pub fn fmt_hours(secs: f64) -> String {
    let h = secs / 3600.0;
    if h < 10.0 {
        let r = (h * 10.0).round() / 10.0;
        if r.fract() == 0.0 {
            format!("{r:.0} h")
        } else {
            format!("{r:.1} h")
        }
    } else {
        format!("{:.0} h", h.round())
    }
}

fn fmt_count(v: f64) -> String {
    let n = v.round() as i64;
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn fmt_tokens(v: f64) -> String {
    if v >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if v >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v >= 1e3 {
        format!("{:.0}k", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

/// Notification text: "Your week is ready: 32 h screen time, top app Xcode".
/// One headline number; `app_name` turns an app id into its display name.
pub fn notification_text(recap: &Recap, app_name: impl Fn(&str) -> String) -> String {
    let what = match recap.period.kind {
        PeriodKind::Week => "week",
        PeriodKind::Month => "month",
        PeriodKind::Year => "year",
    };
    let mut parts: Vec<String> = Vec::new();
    for s in &recap.slides {
        match s {
            Slide::ScreenTime { total_secs, .. } if parts.is_empty() => {
                parts.push(format!("{} screen time", fmt_hours(*total_secs)));
            }
            Slide::TopApps { apps, .. } => {
                if let Some(a) = apps.first() {
                    parts.push(format!("top app {}", app_name(&a.key)));
                }
            }
            Slide::Git { commits, .. } if parts.is_empty() => {
                parts.push(format!("{} commits", fmt_count(*commits)));
            }
            Slide::Photos { count, .. } if parts.is_empty() => {
                parts.push(format!("{} photos", fmt_count(*count)));
            }
            Slide::Ai { tokens, .. } if parts.is_empty() => {
                parts.push(format!("{} AI tokens", fmt_tokens(*tokens)));
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        parts.push(format!("{} active days", recap.active_days));
    }
    format!("Your {what} is ready: {}", parts.join(", "))
}

// ---------- JSON ----------

fn num(v: f64) -> Value {
    if v.fract() == 0.0 && v.abs() < 9e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

fn keyed(k: &Option<Keyed>) -> Value {
    k.as_ref().map_or(
        Value::Null,
        |k| json!({ "key": k.key, "value": num(k.value) }),
    )
}

fn hours_json(h: &[f64; 24]) -> Value {
    h.iter().map(|x| num(*x)).collect()
}

fn range_json(r: &DateRange) -> Value {
    json!({ "from": r.from.to_string(), "to": r.to.to_string(), "days": r.days() })
}

impl Slide {
    pub fn to_json(&self) -> Value {
        let mut v = match self {
            Self::Intro {
                label,
                days,
                active_days,
                screen_secs,
            } => json!({
                "label": label, "days": days, "active_days": active_days,
                "screen_secs": screen_secs.map(num),
            }),
            Self::ScreenTime {
                total_secs,
                daily_avg_secs,
                screen_days,
                change_pct,
                busiest_weekday,
            } => json!({
                "total_secs": num(*total_secs),
                "daily_avg_secs": num(daily_avg_secs.round()),
                "screen_days": screen_days,
                "change_pct": change_pct,
                "busiest_weekday": busiest_weekday.map(|(w, s)| json!({
                    "weekday": w, "avg_secs": num(s.round()),
                })),
            }),
            Self::TopApps { total_secs, apps } => json!({
                "total_secs": num(*total_secs),
                "apps": apps.iter().map(|a| json!({
                    "key": a.key, "secs": num(a.secs), "share": a.share,
                })).collect::<Vec<_>>(),
            }),
            Self::TimeOfDay {
                hours,
                peak_hour,
                label,
                morning_share,
                night_share,
            } => json!({
                "hours": hours_json(hours), "peak_hour": peak_hour, "label": label.name(),
                "morning_share": morning_share, "night_share": night_share,
            }),
            Self::BusiestDay { date, secs } => json!({
                "date": date.to_string(),
                "weekday": date.weekday().num_days_from_monday(),
                "secs": num(*secs),
            }),
            Self::Records {
                longest_session,
                latest_night,
                streak,
            } => json!({
                "longest_session": longest_session.as_ref().map(|s| json!({
                    "app": s.app, "secs": num(s.secs), "start": s.start.to_rfc3339(),
                })),
                "latest_night": latest_night.as_ref().map(|n| json!({
                    "night": n.night.to_string(), "clock_min": n.clock_min,
                })),
                "streak": streak.map(|s| json!({
                    "days": s.days, "from": s.from.to_string(), "to": s.to.to_string(),
                })),
            }),
            Self::Git {
                commits,
                change_pct,
                top_repo,
                busiest_hour,
                hours,
            } => json!({
                "commits": num(*commits), "change_pct": change_pct,
                "top_repo": keyed(top_repo), "busiest_hour": busiest_hour,
                "hours": hours_json(hours),
            }),
            Self::Photos {
                count,
                change_pct,
                top_camera,
                top_lens,
                top_focal,
            } => json!({
                "count": num(*count), "change_pct": change_pct,
                "top_camera": keyed(top_camera), "top_lens": keyed(top_lens),
                "top_focal": keyed(top_focal),
            }),
            Self::Ai {
                tokens,
                input,
                output,
                change_pct,
                top_tool,
                top_model,
                top_project,
            } => json!({
                "tokens": num(*tokens), "input": num(*input), "output": num(*output),
                "change_pct": change_pct,
                "top_tool": keyed(top_tool), "top_model": keyed(top_model),
                "top_project": keyed(top_project),
            }),
            Self::Disk {
                bytes_written,
                change_pct,
            } => json!({ "bytes_written": num(*bytes_written), "change_pct": change_pct }),
            Self::Summary { items } => json!({
                "items": items.iter().map(|h| json!({
                    "id": h.id, "value": num(h.value), "text": h.text,
                })).collect::<Vec<_>>(),
            }),
        };
        v["kind"] = json!(self.kind());
        v
    }
}

impl Recap {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.period.id(),
            "kind": self.period.kind.name(),
            "label": self.period.label(),
            "range": range_json(&self.range),
            "period": range_json(&self.period.range()),
            "in_progress": self.in_progress,
            "previous": range_json(&self.previous),
            "comparable": self.comparable,
            "active_days": self.active_days,
            "available": self.available,
            "slides": self.slides.iter().map(Slide::to_json).collect::<Vec<_>>(),
        })
    }
}

impl RecapEntry {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.period.id(),
            "kind": self.period.kind.name(),
            "label": self.period.label(),
            "range": range_json(&self.range),
            "in_progress": self.in_progress,
            "active_days": self.active_days,
            "screen_secs": self.screen_secs.map(num),
        })
    }
}

/// `list` as a JSON array.
pub fn list_json(entries: &[RecapEntry]) -> Value {
    Value::Array(entries.iter().map(RecapEntry::to_json).collect())
}
