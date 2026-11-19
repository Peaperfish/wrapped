//! GitHub Copilot CLI. UNTESTED: written from public documentation (no Copilot
//! CLI sessions were available; this Mac has only its config and process
//! logs); see docs/AI_SOURCES.md.
//!
//! `$COPILOT_HOME/session-state/<session>/events.jsonl` (older versions:
//! `session-state/<session>.jsonl`); `COPILOT_HOME` defaults to `~/.copilot`.
//! The SDK documents `assistant.usage` as the per-call token event, but also
//! says it is not replayed on resume, so it may be missing from the file. When
//! a session has no usage events at all it is recorded as one request with no
//! tokens, so the day still shows activity.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Adapter, Env, Request, ScanCx, ToolInfo, list_files, parse_ts_ms, uint};

pub const INFO: ToolInfo = ToolInfo {
    id: "copilot",
    title: "GitHub Copilot CLI",
    tokens: true,
    tested: false,
    note: "Untested: written from documentation. Token events may not be saved to disk; then only sessions are counted.",
};

pub struct Copilot;

impl Adapter for Copilot {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        vec![
            env.path("COPILOT_HOME")
                .unwrap_or_else(|| env.home.join(".copilot")),
        ]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        list_files(&root.join("session-state"), &["jsonl"], 2)
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            let session = if f.file_name().is_some_and(|n| n == "events.jsonl") {
                f.parent().and_then(|d| d.file_name())
            } else {
                f.file_stem()
            }
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
            let mut seen_usage = false;
            let mut first: Option<(i64, String)> = None;
            // Not resumable: the "no usage events at all" decision needs the whole file.
            cx.jsonl(&f, false, |line, em| {
                if !line.contains("\"timestamp\"") {
                    return;
                }
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    em.parse_error();
                    return;
                };
                let Some(time_ms) = v["timestamp"].as_str().and_then(parse_ts_ms) else {
                    return;
                };
                if first.is_none() {
                    first = Some((time_ms, String::new()));
                }
                if v["type"].as_str() != Some("assistant.usage") {
                    return;
                }
                let d = if v["data"].is_object() {
                    &v["data"]
                } else {
                    &v
                };
                let id = v["id"]
                    .as_str()
                    .or_else(|| d["apiCallId"].as_str())
                    .map_or_else(|| format!("{time_ms}"), str::to_string);
                seen_usage = true;
                em.request(Request {
                    tool: INFO.id,
                    id_key: &format!("{session}:{id}"),
                    time_ms,
                    session: &session,
                    model: d["model"].as_str().unwrap_or(""),
                    project: "",
                    input: uint(&d["inputTokens"]),
                    output: uint(&d["outputTokens"]),
                    cache_read: uint(&d["cacheReadTokens"]),
                    cache_write: uint(&d["cacheWriteTokens"]),
                });
            });
            // Sessions only: counted when the file was read to the end.
            if !seen_usage && let Some((time_ms, _)) = first {
                cx.session_only(INFO.id, &session, time_ms, "");
            }
        }
    }
}
