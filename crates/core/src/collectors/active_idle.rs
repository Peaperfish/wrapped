//! Active / idle: away periods (idle, asleep, display off, locked, screensaver).
//!
//! The session key is the reason the period *started* with; a period that
//! starts idle and continues into sleep stays one period keyed `idle`.
//! Nothing is recorded while paused or shutting down.

use std::collections::BTreeMap;

use crate::collector::{
    Agg,
    Collector, CollectorInfo, DayInput, Kind, MetricDef, MetricValue, Sink, Slice, Unit,
};
use crate::event::SystemEvent;
use crate::presence::{Presence, Step};

pub const NAME: &str = "active_idle";

#[derive(Default)]
pub struct ActiveIdle {
    out: Option<Sink>,
    presence: Presence,
    paused: bool,
}

impl ActiveIdle {
    fn apply(&self, out: &Sink, step: &Step) {
        let key = match step.view.away {
            Some(reason) if !self.paused && !step.view.halted => {
                Some(out.open_key().unwrap_or_else(|| reason.key().to_string()))
            }
            _ => None,
        };
        out.set_session(key.as_deref(), step.at);
    }
}

impl Collector for ActiveIdle {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "Active / idle",
            description: "When you were away from the computer (idle, asleep, locked, display off).",
            permissions: "None",
            cost: "One idle check per minute; ~0% CPU. A few KB per day on disk.",
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
            MetricDef { id: "away_secs", title: "Time away", unit: Unit::Seconds, agg: Agg::Sum, key: None },
            MetricDef { id: "away_periods", title: "Times away", unit: Unit::Count, agg: Agg::Sum, key: None },
            MetricDef {
                id: "away_secs_by_reason",
                title: "Time away by reason",
                unit: Unit::Seconds, agg: Agg::Sum,
                key: Some("idle | sleep | display_off | locked | screensaver"),
            },
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        let mut total = 0.0;
        let mut periods = 0u32;
        let mut by_reason: BTreeMap<&str, f64> = BTreeMap::new();
        let mut prev: Option<&Slice> = None;
        for sl in day.slices {
            total += sl.secs();
            *by_reason.entry(sl.key.as_str()).or_default() += sl.secs();
            // A period split only because the UTC offset changed continues the
            // previous one; it is not a new period.
            let continues = prev.is_some_and(|p| p.end == sl.start);
            if sl.starts_session && !continues {
                periods += 1;
            }
            prev = Some(sl);
        }
        let mut out = vec![
            MetricValue::new("away_secs", "", total),
            MetricValue::new("away_periods", "", f64::from(periods)),
        ];
        out.extend(
            by_reason
                .into_iter()
                .map(|(k, v)| MetricValue::new("away_secs_by_reason", k, v)),
        );
        out
    }
}
