//! Gemini CLI. UNTESTED: written from public documentation and third-party
//! parsers (no Gemini CLI logs were available); see docs/AI_SOURCES.md.
//!
//! `<home>/.gemini/tmp/<project>/chats/session-*.json` (one JSON document per
//! session; newer builds are reported to write append-only `.jsonl` instead,
//! both are read). `<home>` is `$GEMINI_CLI_HOME` or `~`. A session has
//! `sessionId` and `messages[]`; `gemini` messages carry `model` and
//! `tokens { input, output, cached, thoughts, tool, total }`. Gemini's input
//! count includes the cached part, and thoughts are output.
//!
//! The project folder is a hash in older builds, so it is only known when the
//! project folder holds a `.project_root` file.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Adapter, Emitter, Env, Request, ScanCx, ToolInfo, list_files, parse_ts_ms, project_name, uint,
};

pub const INFO: ToolInfo = ToolInfo {
    id: "gemini",
    title: "Gemini CLI",
    tokens: true,
    tested: false,
    note: "Untested: written from documentation, no Gemini CLI logs were available.",
};

pub struct Gemini;

/// Largest session document read whole.
const MAX_DOC: u64 = 64 * 1024 * 1024;

impl Adapter for Gemini {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        let home = env
            .path("GEMINI_CLI_HOME")
            .unwrap_or_else(|| env.home.clone());
        vec![home.join(".gemini").join("tmp")]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        list_files(root, &["json", "jsonl"], 4)
            .into_iter()
            .filter(|p| {
                p.parent()
                    .and_then(|d| d.file_name())
                    .is_some_and(|n| n == "chats")
            })
            .collect()
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            // <root>/<project>/chats/<file>
            let project = f
                .parent()
                .and_then(Path::parent)
                .map(root_project_name)
                .unwrap_or_default();
            let stem = f
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            if f.extension().is_some_and(|e| e == "jsonl") {
                let mut session = stem.clone();
                cx.jsonl(&f, true, |line, em| {
                    if !line.contains("\"tokens\"") && !line.contains("\"sessionId\"") {
                        return;
                    }
                    match serde_json::from_str::<Value>(line) {
                        Ok(v) => {
                            if let Some(s) = v["sessionId"].as_str() {
                                session = s.to_string();
                            }
                            message(&v, &session, &project, 0, em);
                        }
                        Err(_) => em.parse_error(),
                    }
                });
            } else {
                cx.whole_file(&f, |em| {
                    let Ok(meta) = std::fs::metadata(&f) else {
                        return false;
                    };
                    if meta.len() > MAX_DOC {
                        return false;
                    }
                    let Ok(text) = std::fs::read_to_string(&f) else {
                        return false;
                    };
                    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
                        em.parse_error();
                        return false;
                    };
                    drop(text);
                    let session = doc["sessionId"].as_str().unwrap_or(&stem).to_string();
                    if let Some(msgs) = doc["messages"].as_array() {
                        for (i, m) in msgs.iter().enumerate() {
                            message(m, &session, &project, i, em);
                        }
                    }
                    true
                });
            }
        }
    }
}

/// The project's folder name from its `.project_root` file, if it has one.
fn root_project_name(dir: &Path) -> String {
    std::fs::read_to_string(dir.join(".project_root"))
        .ok()
        .and_then(|s| s.lines().next().map(project_name))
        .unwrap_or_default()
}

/// One message: counted when it is a `gemini` message with token counts.
fn message(m: &Value, session: &str, project: &str, index: usize, em: &mut Emitter<'_>) {
    if m["type"].as_str() != Some("gemini") {
        return;
    }
    let t = &m["tokens"];
    if !t.is_object() {
        return;
    }
    let Some(time_ms) = m["timestamp"].as_str().and_then(parse_ts_ms) else {
        return;
    };
    let id = m["id"]
        .as_str()
        .map_or_else(|| index.to_string(), str::to_string);
    let cached = uint(&t["cached"]);
    em.request(Request {
        tool: INFO.id,
        id_key: &format!("{session}:{id}"),
        time_ms,
        session,
        model: m["model"].as_str().unwrap_or(""),
        project,
        input: uint(&t["input"]).saturating_sub(cached),
        output: uint(&t["output"]) + uint(&t["thoughts"]),
        cache_read: cached,
        cache_write: 0,
    });
}
