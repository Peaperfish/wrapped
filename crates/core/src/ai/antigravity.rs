//! Google Antigravity CLI (`agy`). TESTED against real logs (see
//! docs/AI_SOURCES.md).
//!
//! `~/.gemini/antigravity-cli/brain/<conversation>/.system_generated/logs/
//! transcript_full.jsonl`, one record per step in order. Steps that called
//! the model carry `input_tokens`, `output_tokens` and `cache_read_tokens`
//! (per step, not cumulative; input excludes cache reads). There is no cache
//! write count and no model name in the transcript. The model id is in the
//! conversation database `conversations/<uuid>.db`: each `gen_metadata` row
//! is a protobuf message with the model id (field 1.19) and that call's token
//! counts (1.4.2 input, 1.4.3 output, 1.4.5 cache read). A transcript step is
//! matched to its row by those three counts; a step that has no unique match
//! gets the conversation's most used model, and `unknown` when the database
//! has no model id. Only those four fields are read from the blobs. The other
//! files in the folder (`transcript.jsonl`, `chunks/`) repeat the same steps
//! and are not read.
//!
//! The project comes from `history.jsonl` (`conversationId` -> `workspace`).
//! That file also holds the user's prompts: each line is parsed, only those
//! two fields are taken and the rest is dropped on the spot.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::proto;
use super::{Adapter, Emitter, Env, Request, ScanCx, ToolInfo, parse_ts_ms, project_name, uint};

pub const INFO: ToolInfo = ToolInfo {
    id: "antigravity",
    title: "Google Antigravity",
    tokens: true,
    tested: true,
    note: "The model comes from the conversation database; there is no cache-write count.",
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
            let models = Models::load(&root.join("conversations").join(format!("{session}.db")));
            cx.jsonl(&f, true, |line, em| {
                parse_line(line, &session, &project, &models, em)
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

/// Models of one conversation, from its database.
#[derive(Default)]
struct Models {
    /// (input, output, cache read) -> model, `None` when rows with the same
    /// counts disagree.
    by_counts: HashMap<(u64, u64, u64), Option<String>>,
    /// The most used model.
    main: Option<String>,
}

impl Models {
    fn load(db: &Path) -> Models {
        if !db.is_file() {
            return Models::default();
        }
        let Ok(conn) = super::cursor::open_read_only(db) else {
            return Models::default();
        };
        let Ok(mut st) = conn.prepare("SELECT data FROM gen_metadata") else {
            return Models::default();
        };
        let Ok(mut rows) = st.query([]) else {
            return Models::default();
        };
        let mut m = Models::default();
        let mut count: BTreeMap<String, u64> = BTreeMap::new();
        while let Ok(Some(row)) = rows.next() {
            let Ok(data) = row.get::<_, Vec<u8>>(0) else {
                continue;
            };
            if let Some((counts, model)) = gen_row(&data) {
                *count.entry(model.clone()).or_default() += 1;
                m.by_counts
                    .entry(counts)
                    .and_modify(|e| {
                        if e.as_deref() != Some(model.as_str()) {
                            *e = None;
                        }
                    })
                    .or_insert(Some(model));
            }
        }
        // Most used; a tie goes to the first name alphabetically.
        m.main = count
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
            .map(|(k, _)| k);
        m
    }

    fn model(&self, counts: (u64, u64, u64)) -> &str {
        self.by_counts
            .get(&counts)
            .and_then(|m| m.as_deref())
            .or(self.main.as_deref())
            .unwrap_or("unknown")
    }
}

/// `((input, output, cache read), model id)` from one `gen_metadata` blob.
fn gen_row(data: &[u8]) -> Option<((u64, u64, u64), String)> {
    let model = proto::find_str(data, &[1, 19])?;
    let ok = !model.is_empty()
        && model.len() <= 64
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
    if !ok {
        return None;
    }
    let n = |f: u32| proto::find_varint(data, &[1, 4, f]).unwrap_or(0);
    Some(((n(2), n(3), n(5)), model.to_string()))
}

fn parse_line(line: &str, session: &str, project: &str, models: &Models, em: &mut Emitter<'_>) {
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
    let (input, output, cache_read) = (
        uint(&v["input_tokens"]),
        uint(&v["output_tokens"]),
        uint(&v["cache_read_tokens"]),
    );
    em.request(Request {
        tool: INFO.id,
        id_key: &format!("{session}:{step}"),
        time_ms,
        session,
        model: models.model((input, output, cache_read)),
        project,
        input,
        output,
        cache_read,
        cache_write: 0,
    });
}
