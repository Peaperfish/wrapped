//! Claude Code. TESTED against real logs (see docs/AI_SOURCES.md).
//!
//! `<config dir>/projects/<encoded cwd>/<session>.jsonl` and
//! `.../<session>/subagents/agent-*.jsonl`; the config dir is
//! `$CLAUDE_CONFIG_DIR` or `~/.claude`. Only `assistant` lines with
//! `message.usage` count. One API response is written as several lines (one
//! per content block) with the same `message.id` and the same usage, and
//! resumed / forked sessions copy earlier lines into new files, so the
//! request id is `message.id`, deduplicated across all files.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Adapter, Emitter, Env, Request, ScanCx, ToolInfo, list_files, parse_ts_ms, project_name, uint,
};

pub const INFO: ToolInfo = ToolInfo {
    id: "claude",
    title: "Claude Code",
    tokens: true,
    tested: true,
    note: "Keeps logs for 30 days by default; Wrapped keeps the numbers it has imported.",
};

pub struct Claude;

impl Adapter for Claude {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        vec![
            env.path("CLAUDE_CONFIG_DIR")
                .unwrap_or_else(|| env.home.join(".claude")),
        ]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        // The config dir holds far more than transcripts: look only in projects/.
        let base = root.join("projects");
        let base = if base.is_dir() {
            base
        } else {
            root.to_path_buf()
        };
        list_files(&base, &["jsonl"], 6)
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            cx.jsonl(&f, true, parse_line);
        }
    }
}

/// One line of a transcript. Only lines that can hold usage are parsed at all.
pub fn parse_line(line: &str, em: &mut Emitter<'_>) {
    if !line.contains("\"usage\"") || !line.contains("\"assistant\"") {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        em.parse_error();
        return;
    };
    if v["type"].as_str() != Some("assistant") {
        return;
    }
    let m = &v["message"];
    let u = &m["usage"];
    if !u.is_object() {
        return;
    }
    let model = m["model"].as_str().unwrap_or("");
    // Locally generated messages, not API requests.
    if model == "<synthetic>" {
        return;
    }
    let Some(id) = m["id"].as_str().or_else(|| v["uuid"].as_str()) else {
        return;
    };
    let Some(time_ms) = v["timestamp"].as_str().and_then(parse_ts_ms) else {
        return;
    };
    em.request(Request {
        tool: INFO.id,
        id_key: id,
        time_ms,
        session: v["sessionId"].as_str().unwrap_or(""),
        model,
        project: &project_name(v["cwd"].as_str().unwrap_or("")),
        input: uint(&u["input_tokens"]),
        output: uint(&u["output_tokens"]),
        cache_read: uint(&u["cache_read_input_tokens"]),
        cache_write: uint(&u["cache_creation_input_tokens"]),
    });
}
