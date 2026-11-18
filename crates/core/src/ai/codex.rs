//! Codex CLI. UNTESTED: written from public documentation and third-party
//! parsers (no Codex logs were available); see docs/AI_SOURCES.md.
//!
//! `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`
//! (`CODEX_HOME` may be a comma-separated list; default `~/.codex`). Lines are
//! `{timestamp, type, payload}`. Token counts are `event_msg` lines whose
//! `payload.type` is `token_count`, with *cumulative* per-session totals in
//! `payload.info.total_token_usage`; each request's tokens are the difference
//! from the previous total. Totals are read from the start of every changed
//! file (positions are not used), so deltas are always right, and a request is
//! identified by its timestamp and totals, so a forked session that replays
//! earlier history does not count it again.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Adapter, Emitter, Env, Request, ScanCx, ToolInfo, folder_name, list_files, parse_ts_ms,
    split_roots, uint,
};

pub const INFO: ToolInfo = ToolInfo {
    id: "codex",
    title: "Codex CLI",
    tokens: true,
    tested: false,
    note: "Untested: written from documentation, no Codex logs were available.",
};

pub struct Codex;

impl Adapter for Codex {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        match env.var("CODEX_HOME") {
            Some(v) => split_roots(v.into()),
            None => vec![env.home.join(".codex")],
        }
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        let sessions = root.join("sessions");
        let base = if sessions.is_dir() {
            sessions
        } else {
            root.to_path_buf()
        };
        list_files(&base, &["jsonl"], 5)
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-"))
            })
            .collect()
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            let stem = f
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let mut st = FileState::new(stem);
            cx.jsonl(&f, false, |line, em| st.line(line, em));
        }
    }
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Totals {
    input: u64,
    cached: u64,
    output: u64,
    total: u64,
}

impl Totals {
    fn from(v: &Value) -> Self {
        Self {
            input: uint(&v["input_tokens"]),
            cached: uint(&v["cached_input_tokens"]),
            output: uint(&v["output_tokens"]),
            total: uint(&v["total_tokens"]),
        }
    }
}

/// Per-file reading state: the session, model and project seen so far and the
/// previous cumulative totals.
struct FileState {
    session: String,
    model: String,
    project: String,
    prev: Totals,
}

impl FileState {
    fn new(fallback_session: String) -> Self {
        Self {
            session: fallback_session,
            model: String::new(),
            project: String::new(),
            prev: Totals::default(),
        }
    }

    fn line(&mut self, line: &str, em: &mut Emitter<'_>) {
        let interesting = line.contains("\"token_count\"")
            || line.contains("\"session_meta\"")
            || line.contains("\"turn_context\"");
        if !interesting {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            em.parse_error();
            return;
        };
        let p = &v["payload"];
        match v["type"].as_str() {
            Some("session_meta") => {
                if let Some(id) = p["id"].as_str() {
                    self.session = id.to_string();
                }
                if let Some(cwd) = p["cwd"].as_str() {
                    self.project = folder_name(cwd);
                }
            }
            Some("turn_context") => {
                if let Some(m) = p["model"].as_str() {
                    self.model = m.to_string();
                }
                if self.project.is_empty()
                    && let Some(cwd) = p["cwd"].as_str()
                {
                    self.project = folder_name(cwd);
                }
            }
            Some("event_msg") if p["type"].as_str() == Some("token_count") => {
                let info = &p["info"];
                if !info.is_object() {
                    return;
                }
                let now = Totals::from(&info["total_token_usage"]);
                if now == self.prev {
                    return; // the same total emitted again
                }
                // A total that went down means the counter restarted.
                let reset = now.input < self.prev.input
                    || now.output < self.prev.output
                    || now.cached < self.prev.cached;
                let base = if reset { Totals::default() } else { self.prev };
                let (d_in, d_cached, d_out) = (
                    now.input - base.input,
                    now.cached - base.cached,
                    now.output - base.output,
                );
                self.prev = now;
                let (Some(ts), Some(time_ms)) = (
                    v["timestamp"].as_str(),
                    v["timestamp"].as_str().and_then(parse_ts_ms),
                ) else {
                    return;
                };
                em.request(Request {
                    tool: INFO.id,
                    id_key: &format!("{ts}:{}:{}:{}", now.total, now.input, now.output),
                    time_ms,
                    session: &self.session,
                    model: &self.model,
                    project: &self.project,
                    // Codex's input count includes the cached part.
                    input: d_in.saturating_sub(d_cached),
                    output: d_out,
                    cache_read: d_cached,
                    cache_write: 0,
                });
            }
            _ => {}
        }
    }
}
