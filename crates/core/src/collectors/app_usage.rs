//! App usage: one session per stretch of time an app is in front while the
//! user is present.
//!
//! A session ends on app switch, idle (backdated to last input), sleep,
//! display sleep, screen lock, the lock screen / screensaver coming to the
//! front, power-off, pause, collector stop, or the app being on the exclude
//! list. Excluded apps leave a gap: nothing about them is recorded.

use std::collections::BTreeMap;

use crate::collector::{
    Agg,
    Collector, CollectorInfo, DayInput, Kind, MetricDef, MetricValue, Sink, Unit,
};
use crate::event::{SystemEvent, away_app_reason};
use crate::presence::{Presence, Step};

pub const NAME: &str = "app_usage";

#[derive(Default)]
pub struct AppUsage {
    out: Option<Sink>,
    presence: Presence,
    paused: bool,
}

impl AppUsage {
    fn apply(&self, out: &Sink, step: &Step) {
        let settings = out.settings();
        let key = if self.paused || !step.view.present() {
            None
        } else {
            step.view
                .front
                .as_ref()
                .filter(|f| away_app_reason(&f.bundle_id).is_none())
                .map(|f| f.key())
                .filter(|k| !settings.is_excluded(k))
        };
        out.set_session(key.as_deref(), step.at);
    }
}

impl Collector for AppUsage {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "App usage",
            description: "Which app is in front, with start and end times. Never window titles, \
                          keystrokes, URLs or content.",
            permissions: "None",
            cost: "Event-driven; ~0% CPU. A few KB per day on disk.",
            default_on: true,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Live
    }

    fn start(&mut self, out: Sink) {
        self.out = Some(out);
        // A pause flag from before a stop is stale: the engine sends `Paused`
        // right after starting a collector if tracking is paused.
        self.paused = false;
    }

    fn on_event(&mut self, ev: &SystemEvent) {
        let Some(out) = self.out.clone() else { return };
        match ev {
            SystemEvent::Paused => self.paused = true,
            SystemEvent::Resumed => self.paused = false,
            _ => {}
        }
        let settings = out.settings();
        for step in self.presence.on_event(ev, out.now(), &settings) {
            self.apply(&out, &step);
        }
    }

    fn stop(&mut self) {
        if let Some(out) = self.out.take() {
            out.set_session(None, out.now());
        }
    }

    fn metrics(&self) -> Vec<MetricDef> {
        vec![
            MetricDef { id: "screen_time_secs", title: "Screen time", unit: Unit::Seconds, agg: Agg::Sum, key: None },
            MetricDef { id: "app_secs", title: "Time per app", unit: Unit::Seconds, agg: Agg::Sum, key: Some("bundle id") },
            MetricDef {
                id: "active_secs_by_hour",
                title: "Active time by hour",
                unit: Unit::Seconds, agg: Agg::Sum,
                key: Some("local hour 00..23"),
            },
            MetricDef { id: "app_switches", title: "App switches", unit: Unit::Count, agg: Agg::Sum, key: None },
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        let mut total = 0.0;
        let mut per_app: BTreeMap<&str, f64> = BTreeMap::new();
        let mut per_hour = [0.0f64; 24];
        let mut switches = 0u32;
        let mut prev: Option<&crate::collector::Slice> = None;
        for sl in day.slices {
            let secs = sl.secs();
            if secs > 0.0 {
                total += secs;
                *per_app.entry(sl.key.as_str()).or_default() += secs;
                for (h, s) in sl.hour_secs() {
                    per_hour[h as usize] += s;
                }
            }
            if let Some(p) = prev
                && sl.starts_session
                && p.end == sl.start
                && p.key != sl.key
            {
                switches += 1;
            }
            prev = Some(sl);
        }
        let mut out = vec![MetricValue::new("screen_time_secs", "", total)];
        out.extend(per_app.into_iter().map(|(k, v)| MetricValue::new("app_secs", k, v)));
        out.extend(
            per_hour
                .iter()
                .enumerate()
                .filter(|(_, v)| **v > 0.0)
                .map(|(h, v)| MetricValue::new("active_secs_by_hour", format!("{h:02}"), *v)),
        );
        out.push(MetricValue::new("app_switches", "", f64::from(switches)));
        out
    }
}
