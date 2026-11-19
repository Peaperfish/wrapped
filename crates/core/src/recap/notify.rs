//! When to announce a new recap (the platform layer posts the notification).
//!
//! * Checked at collector start, then at most hourly, and on the first tick
//!   after a wake: [`check`] is cheap when every toggle is off (no query).
//! * A recap is announced from 09:00 local on the day after its period ends
//!   (a year: from 09:00 on 1 December), once. The last announced period per
//!   kind is stored as `internal.recap_notified.<kind>`.
//! * Only ever the newest ready period: after a week asleep there is one
//!   notification, never a burst of old ones.
//! * The first time a kind is turned on, its baseline is the period before
//!   the newest ready one, so at most that newest recap is announced, no
//!   older ones.

use chrono::NaiveDateTime;

use super::{Period, PeriodKind, Recap, build, due, list, notification_text, notify_baseline};
use crate::collector::Collector;
use crate::settings::Settings;
use crate::stats::Stats;
use crate::store::{Result as StoreResult, Store};
use crate::time::Timestamp;

/// Internal setting holding the last announced period of `kind`.
pub fn last_key(kind: PeriodKind) -> String {
    format!("recap_notified.{}", kind.name())
}

/// Internal setting the platform layer keeps the notification permission in:
/// `granted`, `denied`, `not_determined` or `unavailable`.
pub const PERMISSION_KEY: &str = "notify_permission";

pub fn enabled(settings: &Settings, kind: PeriodKind) -> bool {
    match kind {
        PeriodKind::Week => settings.notify_week,
        PeriodKind::Month => settings.notify_month,
        PeriodKind::Year => settings.notify_year,
    }
}

pub fn any_enabled(settings: &Settings) -> bool {
    PeriodKind::ALL.iter().any(|k| enabled(settings, *k))
}

/// One notification to post.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub period: Period,
    pub title: String,
    pub body: String,
}

fn local(now: Timestamp) -> NaiveDateTime {
    chrono::DateTime::from_timestamp_millis(now.local_ms())
        .map(|d| d.naive_utc())
        .unwrap_or_default()
}

/// Store the baseline of every kind that is on and has none yet (call when
/// a toggle is turned on, and before each check).
pub fn ensure_baselines(store: &Store, settings: &Settings, now: Timestamp) -> StoreResult<()> {
    for kind in PeriodKind::ALL {
        if enabled(settings, kind) && store.internal_value(&last_key(kind))?.is_none() {
            let base = notify_baseline(kind, local(now));
            store.set_internal_value(&last_key(kind), &base.id())?;
        }
    }
    Ok(())
}

/// The recaps to announce now (and mark them announced). `app_name` turns
/// an app id into its display name for the text.
pub fn check(
    store: &Store,
    settings: &Settings,
    collectors: &[Box<dyn Collector>],
    now: Timestamp,
    app_name: impl Fn(&str) -> String,
) -> StoreResult<Vec<Notice>> {
    if !any_enabled(settings) {
        return Ok(Vec::new());
    }
    ensure_baselines(store, settings, now)?;
    let at = local(now);
    let today = at.date();
    let stats = Stats::new(store, collectors, settings);
    let available = list(&stats, today)?;
    let mut out = Vec::new();
    for kind in PeriodKind::ALL {
        if !enabled(settings, kind) {
            continue;
        }
        let last = store
            .internal_value(&last_key(kind))?
            .and_then(|v| Period::parse_id(&v));
        let Some(p) = due(kind, at, last, &available) else {
            continue;
        };
        if let Some(r) = build(&stats, p, today)? {
            out.push(notice(&r, &app_name));
        }
        store.set_internal_value(&last_key(kind), &p.id())?;
    }
    Ok(out)
}

/// The notification for a recap (also used for a test notification).
pub fn notice(r: &Recap, app_name: &impl Fn(&str) -> String) -> Notice {
    Notice {
        period: r.period,
        title: "Wrapped".to_string(),
        body: notification_text(r, app_name),
    }
}
