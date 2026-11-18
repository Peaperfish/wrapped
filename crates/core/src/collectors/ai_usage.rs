//! AI usage: token counts from the local logs of AI coding tools.
//!
//! The reading is in [`crate::ai`] (adapters per tool, sources, dedupe); this
//! file is the collector wrapper: what it is called, which metrics it adds and
//! how a day's stored buckets become daily metrics.
//!
//! Stored per request (summed per UTC hour, tool, session, model and project):
//! time, tool, model, input / output / cache-read / cache-write tokens,
//! session id and the project folder *name*. Never prompt or response text,
//! never a full path, never a cost.

use std::collections::{BTreeMap, BTreeSet};

use crate::ai::{self, Env};
use crate::collector::{
    Agg, Collector, CollectorInfo, DayInput, Kind, MetricDef, MetricValue, ScanHooks, Sink, Unit,
};
use crate::settings::{KEY_AI_SOURCES, Settings};

pub const NAME: &str = "ai_usage";

#[derive(Default)]
pub struct AiUsage {
    /// Where tools are looked for (`None`: this machine's real environment).
    /// Tests give it a scratch home so they never read real logs.
    env: Option<Env>,
}

impl AiUsage {
    pub fn with_env(env: Env) -> Self {
        Self { env: Some(env) }
    }
    fn env(&self) -> Env {
        self.env.clone().unwrap_or_else(Env::system)
    }
}

impl Collector for AiUsage {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "AI usage",
            description: "Token counts, model, time, session id and project folder name from the \
                          local logs of AI coding tools (Claude Code, Codex, Gemini CLI, Google \
                          Antigravity, GitHub Copilot CLI, Cursor CLI, opencode). Never prompts, \
                          responses or file contents.",
            permissions: "None (reads the tools' own log folders)",
            cost: "A scan at start and once a day at background priority; later scans read only \
                   what changed.",
            default_on: true,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Scanned
    }

    fn start(&mut self, _out: Sink) {}

    fn stop(&mut self) {}

    fn scanner(&self) -> Option<Box<dyn Collector>> {
        Some(Box::new(AiUsage {
            env: self.env.clone(),
        }))
    }

    /// Tools found on this machine become sources (once); a source whose
    /// folder moved is found again through its bookmark.
    fn prepare_scan(&self, settings: &Settings, hooks: &ScanHooks) -> Vec<(String, String)> {
        match ai::prepare_sources(settings, hooks, &self.env()) {
            Some(sources) => {
                let mut s = settings.clone();
                s.ai_sources = sources;
                vec![(
                    KEY_AI_SOURCES.to_string(),
                    s.get_json(KEY_AI_SOURCES).expect("known key"),
                )]
            }
            None => Vec::new(),
        }
    }

    fn scan(&mut self, _since: Option<crate::time::Timestamp>, out: Sink) {
        ai::scan_sources(&out);
    }

    fn metrics(&self) -> Vec<MetricDef> {
        let count = |id, title, key| MetricDef {
            id,
            title,
            unit: Unit::Count,
            agg: Agg::Sum,
            key,
        };
        vec![
            count("tokens", "AI tokens (input + output)", None),
            count("requests", "AI requests", None),
            count("sessions", "AI sessions", None),
            count(
                "tokens_by_kind",
                "AI tokens by kind",
                Some("input, output, cache_read or cache_write"),
            ),
            count("tokens_by_tool", "AI tokens by tool", Some("tool id")),
            count("requests_by_tool", "AI requests by tool", Some("tool id")),
            count("tokens_by_model", "AI tokens by model", Some("model")),
            count(
                "tokens_by_project",
                "AI tokens by project",
                Some("project folder name"),
            ),
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        if day.ai.is_empty() {
            return Vec::new();
        }
        let (mut tokens, mut requests) = (0u64, 0u64);
        let mut kinds = [0u64; 4];
        let mut sessions: BTreeSet<(&str, &str)> = BTreeSet::new();
        let mut by_tool: BTreeMap<&str, u64> = BTreeMap::new();
        let mut req_by_tool: BTreeMap<&str, u64> = BTreeMap::new();
        let mut by_model: BTreeMap<&str, u64> = BTreeMap::new();
        let mut by_project: BTreeMap<&str, u64> = BTreeMap::new();
        for r in day.ai {
            let t = r.input + r.output;
            tokens += t;
            requests += r.requests;
            for (k, v) in kinds
                .iter_mut()
                .zip([r.input, r.output, r.cache_read, r.cache_write])
            {
                *k += v;
            }
            sessions.insert((&r.tool, &r.session));
            *by_tool.entry(&r.tool).or_default() += t;
            *req_by_tool.entry(&r.tool).or_default() += r.requests;
            if t > 0 {
                *by_model.entry(&r.model).or_default() += t;
                if !r.project.is_empty() {
                    *by_project.entry(&r.project).or_default() += t;
                }
            }
        }
        let mut out = vec![
            MetricValue::new("tokens", "", tokens as f64),
            MetricValue::new("requests", "", requests as f64),
            MetricValue::new("sessions", "", sessions.len() as f64),
        ];
        for (name, v) in ["input", "output", "cache_read", "cache_write"]
            .into_iter()
            .zip(kinds)
        {
            if v > 0 {
                out.push(MetricValue::new("tokens_by_kind", name, v as f64));
            }
        }
        for (k, v) in by_tool {
            if v > 0 {
                out.push(MetricValue::new("tokens_by_tool", k, v as f64));
            }
        }
        for (k, v) in req_by_tool {
            out.push(MetricValue::new("requests_by_tool", k, v as f64));
        }
        for (k, v) in by_model {
            out.push(MetricValue::new("tokens_by_model", k, v as f64));
        }
        for (k, v) in by_project {
            out.push(MetricValue::new("tokens_by_project", k, v as f64));
        }
        out
    }
}
