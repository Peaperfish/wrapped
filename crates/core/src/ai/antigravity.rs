//! Google Antigravity CLI (`agy`). TESTED against real logs (see
//! docs/AI_SOURCES.md).
//!
//! `~/.gemini/antigravity-cli/brain/<conversation>/.system_generated/logs/
//! transcript_full.jsonl`, one record per step in order. Steps that called
//! the model carry `input_tokens`, `output_tokens` and `cache_read_tokens`
//! (per step, not cumulative; input excludes cache reads). There is no cache
//! write count and no model name in the transcript (the model id only appears
//! inside binary blobs in the conversation database, which are not read), so
//! the model is `unknown`. The other files in the folder (`transcript.jsonl`,
//! `chunks/`) repeat the same steps and are not read.
//!
//! The project comes from `history.jsonl` (`conversationId` -> `workspace`).
//! That file also holds the user's prompts: each line is parsed, only those
//! two fields are taken and the rest is dropped on the spot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Adapter, Emitter, Env, Request, ScanCx, ToolInfo, parse_ts_ms, project_name, uint};

pub const INFO: ToolInfo = ToolInfo {
    id: "antigravity",
    title: "Google Antigravity",
    tokens: true,
    tested: true,
    note: "The log has no model name (shown as unknown) and no cache-write count.",
};

pub struct Antigravity;

const LOG: &str = ".system_generated/logs/transcript_full.jsonl";

impl Adapter for Antigravity {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        vec![env.home.join(".gemini").join("antigravity-cli")]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        let Ok(rd) = std::fs::read_dir(root.join("brain")) else {
            return Vec::new();
        };
        let mut v: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path().join(LOG))
            .filter(|p| p.is_file())
            .collect();
        v.sort();
        v
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        let mut projects: BTreeMap<String, String> = BTreeMap::new();
        cx.plain_lines(&root.join("history.jsonl"), |line| {
            if let Some((id, ws)) = history_entry(line) {
                projects.insert(id, ws);
            }
        });
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            // .../brain/<conversation>/.system_generated/logs/transcript_full.jsonl
            let session = f
                .ancestors()
                .nth(3)
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let project = projects.get(&session).cloned().unwrap_or_default();
            cx.jsonl(&f, true, |line, em| {
                parse_line(line, &session, &project, em)
            });
        }
    }
}

/// `(conversationId, project folder name)` from one `history.jsonl` line.
fn history_entry(line: &str) -> Option<(String, String)> {
    let v: Value = serde_json::from_str(line).ok()?;
    let id = v["conversationId"].as_str()?;
    let ws = v["workspace"].as_str()?;
    Some((id.to_string(), project_name(ws)))
}

fn parse_line(line: &str, session: &str, project: &str, em: &mut Emitter<'_>) {
    if !line.contains("\"input_tokens\"") {
        return;
    }
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        em.parse_error();
        return;
    };
    if !v["input_tokens"].is_u64() {
        return;
    }
    let (Some(step), Some(time_ms)) = (
        v["step_index"].as_u64(),
        v["created_at"].as_str().and_then(parse_ts_ms),
    ) else {
        return;
    };
    em.request(Request {
        tool: INFO.id,
        id_key: &format!("{session}:{step}"),
        time_ms,
        session,
        model: "unknown",
        project,
        input: uint(&v["input_tokens"]),
        output: uint(&v["output_tokens"]),
        cache_read: uint(&v["cache_read_tokens"]),
        cache_write: 0,
    });
}
