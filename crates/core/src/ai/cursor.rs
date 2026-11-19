//! Cursor CLI (`cursor-agent`). UNTESTED: written from public documentation and
//! forum posts (no Cursor CLI chats were available); see docs/AI_SOURCES.md.
//!
//! Chats are `~/.cursor/chats/<workspace hash>/<chat id>/store.db` (SQLite: a
//! `meta` table of hex-encoded JSON and content-addressed message blobs).
//! Cursor does **not** save token counts locally, so this records sessions
//! only: one request with no tokens per chat, at its creation time, with the
//! model it last used. Nothing from the message blobs is read.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::{Adapter, Emitter, Env, Request, ScanCx, ToolInfo, hex_decode, list_files};

pub const INFO: ToolInfo = ToolInfo {
    id: "cursor",
    title: "Cursor CLI",
    tokens: false,
    tested: false,
    note: "Cursor does not save token counts on disk: only sessions, time and model are recorded. Untested.",
};

pub struct Cursor;

impl Adapter for Cursor {
    fn info(&self) -> &'static ToolInfo {
        &INFO
    }

    fn default_roots(&self, env: &Env) -> Vec<PathBuf> {
        vec![env.home.join(".cursor").join("chats")]
    }

    fn files(&self, root: &Path) -> Vec<PathBuf> {
        list_files(root, &["db"], 3)
            .into_iter()
            .filter(|p| p.file_name().is_some_and(|n| n == "store.db"))
            .collect()
    }

    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>) {
        for f in self.files(root) {
            if cx.cancelled() {
                return;
            }
            let chat = f
                .parent()
                .and_then(|d| d.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            cx.whole_file(&f, |em| read_chat(&f, &chat, em));
        }
    }
}

/// Open read-only and immutable, so a running tool's database is never touched.
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    let mut uri = String::from("file:");
    for c in path.to_string_lossy().chars() {
        match c {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            ' ' => uri.push_str("%20"),
            c => uri.push(c),
        }
    }
    uri.push_str("?mode=ro&immutable=1");
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
}

/// `(created ms, last used model)` from the chat's `meta` rows, whichever
/// holds them. Only those two fields are taken.
fn meta_fields(conn: &Connection) -> Option<(Option<i64>, String)> {
    let mut st = conn.prepare("SELECT value FROM meta").ok()?;
    let mut rows = st.query([]).ok()?;
    while let Ok(Some(r)) = rows.next() {
        let text: String = match r.get::<_, String>(0) {
            Ok(s) => s,
            Err(_) => match r.get::<_, Vec<u8>>(0) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(_) => continue,
            },
        };
        let json = match hex_decode(text.trim()).and_then(|b| String::from_utf8(b).ok()) {
            Some(decoded) => decoded,
            None => text,
        };
        let Ok(v) = serde_json::from_str::<Value>(&json) else {
            continue;
        };
        let created = v["createdAt"]
            .as_i64()
            .map(|t| if t < 100_000_000_000 { t * 1000 } else { t });
        let model = v["lastUsedModel"].as_str().unwrap_or("").to_string();
        if created.is_some() || !model.is_empty() {
            return Some((created, model));
        }
    }
    None
}

fn read_chat(path: &Path, chat: &str, em: &mut Emitter<'_>) -> bool {
    let Ok(conn) = open_read_only(path) else {
        return false;
    };
    let (created, model) = meta_fields(&conn).unwrap_or((None, String::new()));
    let time_ms = created.or_else(|| {
        std::fs::metadata(path)
            .ok()?
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_millis() as i64)
    });
    let Some(time_ms) = time_ms else { return false };
    em.request(Request {
        tool: INFO.id,
        id_key: chat,
        time_ms,
        session: chat,
        model: &model,
        project: "",
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
    });
    true
}
