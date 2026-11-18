//! The stats engine (DESIGN.md §9 "any date range", §11 recaps): everything
//! the dashboard cards and recap slides show, for any inclusive range of local
//! dates. Pure reads of `daily_metrics` and `sessions`; no UI, no platform code.
//!
//! * Days are local calendar dates as stored in `daily_metrics` (each
//!   collector's rollup already assigned every second, byte and item to
//!   exactly one local date), so a range spanning a DST or UTC offset change
//!   sums each day once.
//! * Metrics combine across days as their [`MetricDef::agg`] says: sum, max,
//!   or mean of the days that have a value.
//! * Metrics of collectors that are turned off are hidden, even if their data
//!   was kept.
//! * Session-based records (longest session, latest night) read `sessions`
//!   and use each session's own UTC offset, like the rollups.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::collector::{Agg, Collector, MetricDef, Unit};
use crate::settings::Settings;
use crate::store::{Result as StoreResult, Store};
use crate::time::{MS_PER_DAY, MS_PER_HOUR, Timestamp, date_from_local_ms, local_midnight_ms};

/// Activity before this local hour belongs to the previous day's night.
pub const NIGHT_ENDS_HOUR: i64 = 5;
/// Keys per keyed metric in a summary for `stats range` and the dashboard.
pub const SUMMARY_TOP_N: usize = 8;
/// Margin around a range when selecting sessions by UTC time (offsets span
/// UTC-12..UTC+14).
const OFFSET_MARGIN_MS: i64 = 15 * MS_PER_HOUR;

/// An inclusive range of local dates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// `from` is after `to`.
    Reversed { from: NaiveDate, to: NaiveDate },
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reversed { from, to } => write!(f, "empty range: {from} is after {to}"),
        }
    }
}
impl std::error::Error for RangeError {}

impl DateRange {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Result<Self, RangeError> {
        if from > to {
            return Err(RangeError::Reversed { from, to });
        }
        Ok(Self { from, to })
    }

    pub fn day(d: NaiveDate) -> Self {
        Self { from: d, to: d }
    }

    /// Number of days (at least 1).
    pub fn days(&self) -> i64 {
        (self.to - self.from).num_days() + 1
    }

    /// The range of equal length just before this one.
    pub fn previous(&self) -> Self {
        let n = chrono::Duration::days(self.days());
        Self {
            from: self.from - n,
            to: self.to - n,
        }
    }

    pub fn contains(&self, d: NaiveDate) -> bool {
        self.from <= d && d <= self.to
    }

    /// Every date in the range, in order.
    pub fn dates(&self) -> impl Iterator<Item = NaiveDate> + use<> {
        let to = self.to;
        self.from.iter_days().take_while(move |d| *d <= to)
    }
}

/// A metric of a collector, with its definition.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricRef {
    pub collector: &'static str,
    pub def: MetricDef,
}

/// One combined value over a range.
#[derive(Debug, Clone, PartialEq)]
pub struct Total {
    pub collector: &'static str,
    pub metric: &'static str,
    /// `""` for metrics without keys.
    pub key: String,
    pub value: f64,
    pub unit: Unit,
    pub agg: Agg,
}

/// A range's value against the previous range of equal length.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub collector: &'static str,
    pub metric: &'static str,
    pub unit: Unit,
    pub current: f64,
    pub previous: f64,
    /// `current - previous`.
    pub change: f64,
    /// Relative change in percent. `Some(0.0)` when both are zero, `None`
    /// when only the previous value is zero (no percentage of nothing).
    pub pct: Option<f64>,
}

/// Values per local hour of day, summed over the range.
#[derive(Debug, Clone, PartialEq)]
pub struct HourProfile {
    pub collector: &'static str,
    /// The `*_by_hour` metric it comes from.
    pub metric: &'static str,
    pub unit: Unit,
    pub hours: [f64; 24],
}

/// The longest single app session.
#[derive(Debug, Clone, PartialEq)]
pub struct LongestSession {
    /// App key (bundle id, or name for apps without one).
    pub app: String,
    pub start: Timestamp,
    pub end: Timestamp,
    pub secs: f64,
}

/// The latest moment you were active on one night.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestNight {
    /// The evening's date (activity from 00:00 to 05:00 belongs to the night
    /// before).
    pub night: NaiveDate,
    pub at: Timestamp,
    /// Local clock time as minutes after midnight (can exceed 24 h, e.g.
    /// 26:30 = 02:30 the next morning, so later nights compare higher).
    pub clock_min: i64,
}

/// A run of consecutive days with a non-zero value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Streak {
    pub days: i64,
    pub from: NaiveDate,
    pub to: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Records {
    pub longest_session: Option<LongestSession>,
    pub latest_night: Option<LatestNight>,
    /// Days with screen time.
    pub active_streak: Option<Streak>,
    /// Days with commits.
    pub commit_streak: Option<Streak>,
}

/// Everything for one range (what `stats range` and the dashboard show).
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub range: DateRange,
    pub previous: DateRange,
    pub totals: Vec<Total>,
    pub comparisons: Vec<Comparison>,
    /// Top keys per keyed metric (not `*_by_hour`), best first.
    pub top: Vec<(MetricRef, Vec<(String, f64)>)>,
    pub time_of_day: Vec<HourProfile>,
    /// Per-day values of every metric without keys (zeros for missing days).
    pub series: Vec<(MetricRef, Vec<(NaiveDate, f64)>)>,
    pub records: Records,
}

/// The stats engine over one store.
pub struct Stats<'a> {
    store: &'a Store,
    metrics: Vec<MetricRef>,
}

/// One stored `daily_metrics` row, already matched to its definition.
struct Row {
    m: usize,
    date: NaiveDate,
    key: String,
    value: f64,
}

/// Running combination of daily values.
#[derive(Default, Clone, Copy)]
struct Acc {
    sum: f64,
    max: f64,
    n: u32,
}

impl Acc {
    fn add(&mut self, v: f64) {
        self.max = if self.n == 0 { v } else { self.max.max(v) };
        self.sum += v;
        self.n += 1;
    }
    fn get(&self, agg: Agg) -> f64 {
        match agg {
            Agg::Sum => self.sum,
            Agg::Max => self.max,
            Agg::Mean if self.n > 0 => self.sum / f64::from(self.n),
            Agg::Mean => 0.0,
        }
    }
}

impl<'a> Stats<'a> {
    /// Stats for the collectors that are turned on in `settings`.
    pub fn new(store: &'a Store, collectors: &[Box<dyn Collector>], settings: &Settings) -> Self {
        let metrics = collectors
            .iter()
            .filter(|c| {
                let i = c.info();
                settings.collector_enabled(i.name, i.default_on)
            })
            .flat_map(|c| {
                let name = c.info().name;
                c.metrics().into_iter().map(move |def| MetricRef {
                    collector: name,
                    def,
                })
            })
            .collect();
        Self { store, metrics }
    }

    /// The visible metrics (collectors that are on), in collector order.
    pub fn metrics(&self) -> &[MetricRef] {
        &self.metrics
    }

    pub fn collector_visible(&self, collector: &str) -> bool {
        self.metrics.iter().any(|m| m.collector == collector)
    }

    fn find(&self, collector: &str, metric: &str) -> Option<usize> {
        self.metrics
            .iter()
            .position(|m| m.collector == collector && m.def.id == metric)
    }

    /// Visible rows in the range (one SQL range scan on the primary key).
    fn rows(&self, r: DateRange, only: Option<(&str, &str)>) -> StoreResult<Vec<Row>> {
        let mut out = Vec::new();
        self.store.each_daily_metric_between(
            r.from,
            r.to,
            only,
            |date, c, metric, key, value| {
                if let Some(m) = self.find(c, metric) {
                    out.push(Row {
                        m,
                        date,
                        key: key.to_string(),
                        value,
                    });
                }
            },
        )?;
        Ok(out)
    }

    fn combine(&self, rows: &[Row]) -> BTreeMap<(usize, String), Acc> {
        let mut acc: BTreeMap<(usize, String), Acc> = BTreeMap::new();
        for r in rows {
            acc.entry((r.m, r.key.clone())).or_default().add(r.value);
        }
        acc
    }

    /// Every visible metric and key with data in the range, combined across
    /// days by its aggregation.
    pub fn totals(&self, r: DateRange) -> StoreResult<Vec<Total>> {
        Ok(self.totals_of(&self.combine(&self.rows(r, None)?)))
    }

    fn totals_of(&self, acc: &BTreeMap<(usize, String), Acc>) -> Vec<Total> {
        acc.iter()
            .map(|((m, key), a)| {
                let mr = &self.metrics[*m];
                Total {
                    collector: mr.collector,
                    metric: mr.def.id,
                    key: key.clone(),
                    value: a.get(mr.def.agg),
                    unit: mr.def.unit,
                    agg: mr.def.agg,
                }
            })
            .collect()
    }

    /// One value per day of the range (zeros for days without data), for the
    /// heatmap. `key`: one key of a keyed metric, or `None` for the metric
    /// without keys / all keys combined per day by the metric's aggregation.
    /// Empty if the metric is unknown or hidden.
    pub fn series(
        &self,
        collector: &str,
        metric: &str,
        key: Option<&str>,
        r: DateRange,
    ) -> StoreResult<Vec<(NaiveDate, f64)>> {
        let Some(m) = self.find(collector, metric) else {
            return Ok(Vec::new());
        };
        let agg = self.metrics[m].def.agg;
        let mut per_day: BTreeMap<NaiveDate, Acc> = BTreeMap::new();
        for row in self.rows(r, Some((collector, metric)))? {
            if key.is_none_or(|k| k == row.key) {
                per_day.entry(row.date).or_default().add(row.value);
            }
        }
        Ok(r.dates()
            .map(|d| (d, per_day.get(&d).map_or(0.0, |a| a.get(agg))))
            .collect())
    }

    /// The `n` keys with the highest combined value, best first (ties by key).
    pub fn top(
        &self,
        collector: &str,
        metric: &str,
        r: DateRange,
        n: usize,
    ) -> StoreResult<Vec<(String, f64)>> {
        let Some(m) = self.find(collector, metric) else {
            return Ok(Vec::new());
        };
        let agg = self.metrics[m].def.agg;
        let acc = self.combine(&self.rows(r, Some((collector, metric)))?);
        Ok(top_of(&acc, m, agg, n))
    }

    /// Range totals of every metric without keys against the previous range
    /// of equal length. Metrics with no data in either range are left out.
    pub fn compare(&self, r: DateRange) -> StoreResult<Vec<Comparison>> {
        let cur = self.combine(&self.rows(r, None)?);
        let prev = self.combine(&self.rows(r.previous(), None)?);
        Ok(self.comparisons_of(&cur, &prev))
    }

    fn comparisons_of(
        &self,
        cur: &BTreeMap<(usize, String), Acc>,
        prev: &BTreeMap<(usize, String), Acc>,
    ) -> Vec<Comparison> {
        let mut out = Vec::new();
        for (m, mr) in self.metrics.iter().enumerate() {
            if mr.def.key.is_some() {
                continue;
            }
            let k = (m, String::new());
            let (c, p) = (cur.get(&k), prev.get(&k));
            if c.is_none() && p.is_none() {
                continue;
            }
            let current = c.map_or(0.0, |a| a.get(mr.def.agg));
            let previous = p.map_or(0.0, |a| a.get(mr.def.agg));
            out.push(Comparison {
                collector: mr.collector,
                metric: mr.def.id,
                unit: mr.def.unit,
                current,
                previous,
                change: current - previous,
                pct: pct_change(current, previous),
            });
        }
        out
    }

    /// Sums per local hour of every visible `*_by_hour` metric (keys "00".."23").
    pub fn time_of_day(&self, r: DateRange) -> StoreResult<Vec<HourProfile>> {
        Ok(self.profiles_of(&self.combine(&self.rows(r, None)?)))
    }

    fn profiles_of(&self, acc: &BTreeMap<(usize, String), Acc>) -> Vec<HourProfile> {
        let mut out = Vec::new();
        for (m, mr) in self.metrics.iter().enumerate() {
            if !is_by_hour(&mr.def) {
                continue;
            }
            let mut hours = [0.0; 24];
            for ((am, key), a) in acc.range((m, String::new())..(m + 1, String::new())) {
                debug_assert_eq!(*am, m);
                if let Ok(h) = key.parse::<usize>()
                    && h < 24
                {
                    hours[h] = a.get(mr.def.agg);
                }
            }
            out.push(HourProfile {
                collector: mr.collector,
                metric: mr.def.id,
                unit: mr.def.unit,
                hours,
            });
        }
        out
    }

    /// Records for the range (each `None` if its collector is off or there
    /// is no data).
    pub fn records(&self, r: DateRange) -> StoreResult<Records> {
        let mut rec = Records::default();
        if self.collector_visible("app_usage") {
            let (longest, night) = self.session_records(r)?;
            rec.longest_session = longest;
            rec.latest_night = night;
            rec.active_streak = streak(&self.series("app_usage", "screen_time_secs", None, r)?);
        }
        if self.collector_visible("git") {
            rec.commit_streak = streak(&self.series("git", "commits", None, r)?);
        }
        Ok(rec)
    }

    /// Longest app session starting in the range (pieces split only by a UTC
    /// offset change count as one) and the latest night in the range.
    fn session_records(
        &self,
        r: DateRange,
    ) -> StoreResult<(Option<LongestSession>, Option<LatestNight>)> {
        let lo = local_midnight_ms(r.from) - OFFSET_MARGIN_MS - self.store.max_session_ms();
        let hi =
            local_midnight_ms(r.to) + MS_PER_DAY + NIGHT_ENDS_HOUR * MS_PER_HOUR + OFFSET_MARGIN_MS;
        let night_shift = NIGHT_ENDS_HOUR * MS_PER_HOUR;
        let mut longest: Option<LongestSession> = None;
        let mut latest: Option<LatestNight> = None;
        let mut run: Option<LongestSession> = None;
        let consider = |s: LongestSession, longest: &mut Option<LongestSession>| {
            if r.contains(s.start.local_date()) && longest.as_ref().is_none_or(|l| s.secs > l.secs)
            {
                *longest = Some(s);
            }
        };
        self.store
            .each_session_of("app_usage", lo, hi, |key, start, end| {
                if end.utc_ms <= start.utc_ms {
                    return;
                }
                // Longest session: join an offset-change split to its first piece.
                match &mut run {
                    Some(cur) if cur.app == key && cur.end.utc_ms == start.utc_ms => {
                        cur.end = end;
                        cur.secs = (end.utc_ms - cur.start.utc_ms) as f64 / 1000.0;
                    }
                    _ => {
                        if let Some(prev) = run.take() {
                            consider(prev, &mut longest);
                        }
                        run = Some(LongestSession {
                            app: key.to_string(),
                            start,
                            end,
                            secs: (end.utc_ms - start.utc_ms) as f64 / 1000.0,
                        });
                    }
                }
                // Latest night: in the session's start offset, shift the clock
                // back 5 h so a "night" is one shifted day.
                let off = start.local_ms() - start.utc_ms;
                let (s, e) = (
                    start.local_ms() - night_shift,
                    end.utc_ms + off - night_shift,
                );
                let mut day0 = s - s.rem_euclid(MS_PER_DAY);
                while day0 < e {
                    let night = date_from_local_ms(day0);
                    let last = e.min(day0 + MS_PER_DAY);
                    if r.contains(night) {
                        let clock_min = (last - day0 + night_shift) / 60_000;
                        if latest.as_ref().is_none_or(|l| clock_min > l.clock_min) {
                            latest = Some(LatestNight {
                                night,
                                at: Timestamp::new(last + night_shift - off, start.offset_min),
                                clock_min,
                            });
                        }
                    }
                    day0 += MS_PER_DAY;
                }
            })?;
        if let Some(prev) = run.take() {
            consider(prev, &mut longest);
        }
        Ok((longest, latest))
    }

    /// Everything for one range, compared with the previous range of equal
    /// length; `top_n` keys per keyed metric.
    pub fn summary(&self, r: DateRange, top_n: usize) -> StoreResult<Summary> {
        self.summary_vs(r, None, top_n)
    }

    /// Like [`Stats::summary`], compared with `vs` instead of the previous
    /// range of equal length (for a period still in progress: the same
    /// elapsed days of the period before). `None` is `summary`.
    pub fn summary_vs(
        &self,
        r: DateRange,
        vs: Option<DateRange>,
        top_n: usize,
    ) -> StoreResult<Summary> {
        let vs = vs.unwrap_or_else(|| r.previous());
        let rows = self.rows(r, None)?;
        let cur = self.combine(&rows);
        let prev = self.combine(&self.rows(vs, None)?);
        let mut top = Vec::new();
        let mut series = Vec::new();
        let mut per_day: BTreeMap<(usize, NaiveDate), f64> = BTreeMap::new();
        for row in rows.iter().filter(|row| row.key.is_empty()) {
            per_day.insert((row.m, row.date), row.value);
        }
        for (m, mr) in self.metrics.iter().enumerate() {
            if mr.def.key.is_none() {
                series.push((
                    mr.clone(),
                    r.dates()
                        .map(|d| (d, per_day.get(&(m, d)).copied().unwrap_or(0.0)))
                        .collect(),
                ));
            } else if !is_by_hour(&mr.def) {
                let friendly: Option<fn(&str) -> String> = match (mr.collector, mr.def.id) {
                    ("photos", "shots_by_camera") => {
                        Some(crate::collectors::photos::friendly_camera)
                    }
                    ("photos", "shots_by_lens") => Some(crate::collectors::photos::friendly_lens),
                    _ => None,
                };
                let keys = if let Some(f) = friendly {
                    friendly_keys(top_of(&cur, m, mr.def.agg, usize::MAX), top_n, f)
                } else {
                    top_of(&cur, m, mr.def.agg, top_n)
                };
                top.push((mr.clone(), keys));
            }
        }
        Ok(Summary {
            range: r,
            previous: vs,
            totals: self.totals_of(&cur),
            comparisons: self.comparisons_of(&cur, &prev),
            top,
            time_of_day: self.profiles_of(&cur),
            series,
            records: self.records(r)?,
        })
    }
}

/// Keys by their friendly names (cameras, lenses); keys that share one name (a
/// drone's two cameras) add up. Best first, at most `n`.
fn friendly_keys(
    keys: Vec<(String, f64)>,
    n: usize,
    friendly: fn(&str) -> String,
) -> Vec<(String, f64)> {
    let mut merged: BTreeMap<String, f64> = BTreeMap::new();
    for (label, v) in keys {
        *merged.entry(friendly(&label)).or_default() += v;
    }
    let mut v: Vec<(String, f64)> = merged.into_iter().collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

fn is_by_hour(def: &MetricDef) -> bool {
    def.key.is_some() && def.id.ends_with("_by_hour")
}

fn top_of(
    acc: &BTreeMap<(usize, String), Acc>,
    m: usize,
    agg: Agg,
    n: usize,
) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = acc
        .range((m, String::new())..(m + 1, String::new()))
        .map(|((_, k), a)| (k.clone(), a.get(agg)))
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

/// Percent change from `previous` to `current` (see [`Comparison::pct`]).
pub fn pct_change(current: f64, previous: f64) -> Option<f64> {
    if previous == 0.0 {
        (current == 0.0).then_some(0.0)
    } else {
        Some((current - previous) / previous.abs() * 100.0)
    }
}

/// Longest run of consecutive days with a value above zero (the earliest if
/// tied).
pub fn streak(series: &[(NaiveDate, f64)]) -> Option<Streak> {
    let mut best: Option<Streak> = None;
    let mut cur: Option<Streak> = None;
    for (d, v) in series {
        if *v > 0.0 {
            cur = Some(match cur {
                Some(s) if s.to.succ_opt() == Some(*d) => Streak {
                    days: s.days + 1,
                    to: *d,
                    ..s
                },
                _ => Streak {
                    days: 1,
                    from: *d,
                    to: *d,
                },
            });
            if let Some(c) = cur
                && best.is_none_or(|b| c.days > b.days)
            {
                best = Some(c);
            }
        } else {
            cur = None;
        }
    }
    best
}

// ---------- JSON (for the dashboard) ----------

fn ts_json(t: Timestamp) -> Value {
    json!({ "utc_ms": t.utc_ms, "offset_min": t.offset_min, "local": t.to_rfc3339() })
}

fn streak_json(s: &Option<Streak>) -> Value {
    s.map_or(
        Value::Null,
        |s| json!({ "days": s.days, "from": s.from.to_string(), "to": s.to.to_string() }),
    )
}

fn num(v: f64) -> Value {
    // Whole numbers print without ".0".
    if v.fract() == 0.0 && v.abs() < 9e15 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

impl Summary {
    pub fn to_json(&self) -> Value {
        let range = |r: &DateRange| json!({ "from": r.from.to_string(), "to": r.to.to_string(), "days": r.days() });
        let mref = |m: &MetricRef| {
            json!({
                "collector": m.collector,
                "metric": m.def.id,
                "title": m.def.title,
                "unit": m.def.unit.name(),
                "agg": m.def.agg.name(),
            })
        };
        let rec = &self.records;
        json!({
            "range": range(&self.range),
            "previous": range(&self.previous),
            "totals": self.totals.iter().map(|t| json!({
                "collector": t.collector, "metric": t.metric, "key": t.key,
                "value": num(t.value), "unit": t.unit.name(), "agg": t.agg.name(),
            })).collect::<Vec<_>>(),
            "comparisons": self.comparisons.iter().map(|c| json!({
                "collector": c.collector, "metric": c.metric, "unit": c.unit.name(),
                "current": num(c.current), "previous": num(c.previous),
                "change": num(c.change), "pct": c.pct,
            })).collect::<Vec<_>>(),
            "top": self.top.iter().map(|(m, keys)| {
                let mut v = mref(m);
                v["keys"] = keys.iter().map(|(k, x)| json!({ "key": k, "value": num(*x) })).collect();
                v
            }).collect::<Vec<_>>(),
            "time_of_day": self.time_of_day.iter().map(|p| json!({
                "collector": p.collector, "metric": p.metric, "unit": p.unit.name(),
                "hours": p.hours.iter().map(|h| num(*h)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "series": self.series.iter().map(|(m, days)| {
                let mut v = mref(m);
                v["days"] = days.iter().map(|(_, x)| num(*x)).collect();
                v
            }).collect::<Vec<_>>(),
            "records": {
                "longest_session": rec.longest_session.as_ref().map(|s| json!({
                    "app": s.app, "start": ts_json(s.start), "end": ts_json(s.end), "secs": num(s.secs),
                })),
                "latest_night": rec.latest_night.as_ref().map(|n| json!({
                    "night": n.night.to_string(), "at": ts_json(n.at), "clock_min": n.clock_min,
                })),
                "active_streak": streak_json(&rec.active_streak),
                "commit_streak": streak_json(&rec.commit_streak),
            },
        })
    }
}

/// Collectors whose data is in the store but hidden (turned off), for messages.
pub fn hidden_collectors(
    collectors: &[Box<dyn Collector>],
    settings: &Settings,
) -> BTreeSet<&'static str> {
    collectors
        .iter()
        .map(|c| c.info())
        .filter(|i| !settings.collector_enabled(i.name, i.default_on))
        .map(|i| i.name)
        .collect()
}
