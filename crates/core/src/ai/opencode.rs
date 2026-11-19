//! opencode. UNTESTED: written from public documentation and third-party
//! parsers (no opencode data was available); see docs/AI_SOURCES.md.
//!
//! `$OPENCODE_DATA_DIR`, else `$XDG_DATA_HOME/opencode`, else
//! `~/.local/share/opencode`. Message files are
//! `storage/message/<session>/<message>.json`; some versions keep them in a
//! SQLite database (`opencode.db`, table `message`, JSON in `data`), which is
//! read too. An assistant message has `modelID`, `time.created` /
//! `time.completed` (Unix ms), `path.cwd` and
//! `tokens { input, output, reasoning, cache { read, write } }`. A message is
//! counted once it has `time.completed` (files are rewritten while streaming).
//! The per-step `step-finish` parts repeat the same tokens and are not read.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::cursor::open_read_only;
use super::{Adapter, Emitter, Env, Request, ScanCx, ToolInfo, folder_name, list_files, uint};

pub const INFO: ToolInfo = ToolInfo {
    id: "opencode",
    title: "opencode",
    tokens: true,
    tested: false,
    note: "Untested: written from documentation, no opencode data was available.",
};

pub struct OpenCode;

impl Adapter for OpenCode {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        let root = env
            .path("OPENCODE_DATA_DIR")
            .or_else(|| env.path("XDG_DATA_HOME").map(|d| d.join("opencode")))
            .unwrap_or_else(|| env.home.join(".local").join("share").join("opencode"));
        vec![root]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        let mut v = list_files(&root.join("storage").join("message"), &["json"], 3);
        let db = root.join("opencode.db");
        if db.is_file() {
            v.push(db);
        }
        v
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            if f.extension().is_some_and(|e| e == "db") {
                cx.whole_file(&f, |em| read_db(&f, em));
            } else {
                cx.whole_file(&f, |em| {
                    let Ok(text) = std::fs::read_to_string(&f) else {
                        return false;
                    };
                    match serde_json::from_str::<Value>(&text) {
                        Ok(v) => {
                            message(&v, "", em);
                            true
                        }
                        Err(_) => {
                            em.parse_error();
                            false
                        }
                    }
                });
            }
        }
    }
}

fn read_db(path: &Path, em: &mut Emitter<'_>) -> bool {
    let Ok(conn) = open_read_only(path) else {
        return false;
    };
    let Ok(mut st) = conn.prepare("SELECT session_id, data FROM message") else {
        return false;
    };
    let Ok(mut rows) = st.query([]) else {
        return false;
    };
    while let Ok(Some(r)) = rows.next() {
        let session: String = r.get(0).unwrap_or_default();
        let data: String = r.get(1).unwrap_or_default();
        match serde_json::from_str::<Value>(&data) {
            Ok(v) => message(&v, &session, em),
            Err(_) => em.parse_error(),
        }
    }
    true
}

/// One message: counted when it is a finished assistant message with tokens.
fn message(m: &Value, session_hint: &str, em: &mut Emitter<'_>) {
    if m["role"].as_str() != Some("assistant") {
        return;
    }
    let t = &m["tokens"];
    let Some(done) = m["time"]["completed"].as_i64() else {
        return;
    };
    if !t.is_object() {
        return;
    }
    let created = m["time"]["created"].as_i64().unwrap_or(done);
    let Some(id) = m["id"].as_str() else { return };
    let session = m["sessionID"].as_str().unwrap_or(session_hint);
    em.request(Request {
        tool: INFO.id,
        id_key: id,
        time_ms: created,
        session,
        model: m["modelID"].as_str().unwrap_or(""),
        project: &folder_name(m["path"]["cwd"].as_str().unwrap_or("")),
        input: uint(&t["input"]),
        output: uint(&t["output"]) + uint(&t["reasoning"]),
        cache_read: uint(&t["cache"]["read"]),
        cache_write: uint(&t["cache"]["write"]),
    });
}
