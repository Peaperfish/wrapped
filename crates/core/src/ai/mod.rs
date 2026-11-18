//! AI usage: token counts from the local logs of AI coding tools.
//!
//! One [`Adapter`] per tool knows where the tool keeps its logs and how to
//! read them (see `docs/AI_SOURCES.md` for what each tool records). The scan
//! driver in this module walks the configured *sources* (a folder plus the
//! tool whose format it holds), calls the adapter, and emits one
//! [`AiRecord`](crate::collector::AiRecord) per request.
//!
//! What is kept, per request: time, tool, model, input / output / cache-read /
//! cache-write tokens, session id and the project folder's *name* (last path
//! component, cut inside the adapter). Never prompt or response text, never a
//! full path, never a cost. Lines are parsed, the whitelisted fields are taken
//! and the parsed value is dropped at once; nothing from a log line is
//! logged or stored beyond those fields.
//!
//! Counting once: every request has a stable id (the tool's own message /
//! step id, or a hash of its cumulative counters); the store keeps a hash of
//! every counted id forever (`ai_seen`), so rescans, resumed or forked
//! sessions and repeated lines never double count. Per file the scan keeps a
//! byte offset and a hash of the file's first line, so a log that only grew is
//! read from where it stopped and one that did not change is not opened.
//!
//! Sources are never forgotten: only an explicit request removes one. A
//! source whose folder is missing is reported as such, checked again on every
//! scan, and picked up where it left off when the folder returns (on macOS,
//! also when it was moved or renamed, through a folder bookmark).

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot;
pub mod cursor;
pub mod gemini;
pub mod lines;
pub mod opencode;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Offset, TimeZone};
use serde_json::{Value, json};

use crate::collector::{AiRecord, FileStamp, ScanHooks, Sink};
use crate::scan;
use crate::settings::{AiSource, Settings, expand_home};
use crate::time::Timestamp;

use lines::{Line, Lines};

/// Longest log line read into memory; longer ones are skipped.
const MAX_LINE: usize = 8 * 1024 * 1024;
/// Check for cancellation every this many lines.
const CANCEL_EVERY: u64 = 512;

/// What is known about a tool.
#[derive(Debug, PartialEq, Eq)]
pub struct ToolInfo {
    /// Stable id stored in the database and settings. Never rename.
    pub id: &'static str,
    pub title: &'static str,
    /// The tool writes token counts to disk. False: only sessions, time and
    /// model are recorded.
    pub tokens: bool,
    /// Checked against real logs on a machine that has the tool. False:
    /// written from the tool's documentation only.
    pub tested: bool,
    /// One line for Settings.
    pub note: &'static str,
}

pub const TOOLS: &[ToolInfo] = &[
    claude::INFO,
    codex::INFO,
    gemini::INFO,
    antigravity::INFO,
    copilot::INFO,
    cursor::INFO,
    opencode::INFO,
];

pub fn tool(id: &str) -> Option<&'static ToolInfo> {
    TOOLS.iter().find(|t| t.id == id)
}

/// How a tool's logs are found and read.
pub trait Adapter: Sync {
    fn info(&self) -> &'static ToolInfo;
    /// The folders this tool uses on this machine by default (env overrides
    /// applied), whether or not they exist.
    fn default_roots(&self, env: &Env) -> Vec<PathBuf>;
    /// The data files under `root`, in a stable order (cheap: no reading).
    fn files(&self, root: &Path) -> Vec<PathBuf>;
    /// Read the new requests under `root`.
    fn scan(&self, root: &Path, cx: &mut ScanCx<'_>);
}

fn adapters() -> [&'static dyn Adapter; 7] {
    [
        &claude::Claude,
        &codex::Codex,
        &gemini::Gemini,
        &antigravity::Antigravity,
        &copilot::Copilot,
        &cursor::Cursor,
        &opencode::OpenCode,
    ]
}

pub fn adapter(tool_id: &str) -> Option<&'static dyn Adapter> {
    adapters().into_iter().find(|a| a.info().id == tool_id)
}

/// Environment variables and the home folder, injectable for tests.
#[derive(Clone)]
pub struct Env {
    vars: std::sync::Arc<dyn Fn(&str) -> Option<String> + Send + Sync>,
    pub home: PathBuf,
}

impl Env {
    pub fn system() -> Self {
        let home = expand_home("~").unwrap_or_else(|| PathBuf::from("."));
        Self {
            vars: std::sync::Arc::new(|k| std::env::var(k).ok().filter(|v| !v.is_empty())),
            home,
        }
    }
    pub fn from_map(home: PathBuf, vars: &[(&str, &str)]) -> Self {
        let m: BTreeMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Self {
            vars: std::sync::Arc::new(move |k| m.get(k).cloned()),
            home,
        }
    }
    pub fn var(&self, name: &str) -> Option<String> {
        (self.vars)(name)
    }
    /// An env var as a path.
    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.var(name).map(PathBuf::from)
    }
}

// ---------- helpers for adapters ----------

/// The last path component of a project path (either separator), or empty.
/// This is the only form of a project folder that ever leaves an adapter.
pub fn folder_name(path: &str) -> String {
    let t = path.trim_end_matches(['/', '\\']);
    let name = t.rsplit(['/', '\\']).next().unwrap_or("");
    name.chars().take(120).collect()
}

/// An RFC 3339 timestamp as UTC milliseconds.
pub fn parse_ts_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|t| t.timestamp_millis())
}

/// This machine's UTC offset in minutes at that instant.
pub fn local_offset_min(utc_ms: i64) -> i32 {
    Local
        .timestamp_millis_opt(utc_ms)
        .single()
        .map_or(0, |t| t.offset().fix().local_minus_utc() / 60)
}

/// A non-negative integer field (missing, negative or not a number = 0).
pub fn uint(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

const ID_KEYS: [u64; 2] = [0x7772_6170_7065_6461, 0x6169_2d72_6571_7569];

/// The dedupe id of a request: a stable hash of the tool and its own id.
pub fn request_id(tool: &str, id_key: &str) -> u64 {
    scan::siphash24(
        ID_KEYS[0],
        ID_KEYS[1],
        format!("{tool}\0{id_key}").as_bytes(),
    )
}

/// Files with one of `exts` under `root`, depth first in name order. Symlinks
/// are not followed; `max_depth` bounds the walk.
pub fn list_files(root: &Path, exts: &[&str], max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        let mut subdirs = Vec::new();
        for e in entries {
            let Ok(t) = e.file_type() else { continue };
            let p = e.path();
            if t.is_dir() {
                if depth < max_depth {
                    subdirs.push(p);
                }
            } else if t.is_file()
                && p.extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| exts.iter().any(|w| x.eq_ignore_ascii_case(w)))
            {
                out.push(p);
            }
        }
        stack.extend(subdirs.into_iter().rev().map(|d| (d, depth + 1)));
    }
    out
}

fn mtime_ms(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// A hash of a file's first line (up to 4 KB), or 0 when the file has no
/// complete first line yet (it is then re-read whole on change).
fn head_hash(file: &mut File) -> u64 {
    let mut buf = [0u8; 4096];
    let mut n = 0;
    while n < buf.len() {
        match file.read(&mut buf[n..]) {
            Ok(0) | Err(_) => break,
            Ok(k) => n += k,
        }
    }
    let _ = file.seek(SeekFrom::Start(0));
    let slice = &buf[..n];
    let line = match slice.iter().position(|&b| b == b'\n') {
        Some(i) => &slice[..i],
        None if n == buf.len() => slice,
        None => return 0,
    };
    scan::siphash24(ID_KEYS[1], ID_KEYS[0], line).max(1)
}

/// What a scan of one source counted (for the report and the status line).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub files: u64,
    pub files_read: u64,
    pub files_unchanged: u64,
    pub lines: u64,
    pub lines_too_long: u64,
    pub lines_invalid: u64,
    pub parse_errors: u64,
    pub requests: u64,
    pub errors: u64,
}

/// Hands requests found by an adapter to the scan.
pub struct Emitter<'a> {
    out: &'a Sink,
    counts: &'a mut Counts,
}

/// The fields of one request, as an adapter found them.
pub struct Request<'a> {
    pub tool: &'static str,
    /// The tool's own id for the request (deduplicated on this).
    pub id_key: &'a str,
    pub time_ms: i64,
    pub session: &'a str,
    pub model: &'a str,
    /// Already reduced to a folder name.
    pub project: &'a str,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

impl Emitter<'_> {
    pub fn request(&mut self, r: Request<'_>) {
        self.counts.requests += 1;
        self.out.ai(AiRecord {
            id: request_id(r.tool, r.id_key),
            time: Timestamp::new(r.time_ms, local_offset_min(r.time_ms)),
            tool: r.tool,
            session: clip(r.session, 80),
            model: if r.model.is_empty() {
                "unknown".to_string()
            } else {
                clip(r.model, 80)
            },
            project: clip(r.project, 120),
            input: r.input,
            output: r.output,
            cache_read: r.cache_read,
            cache_write: r.cache_write,
        });
    }
    pub fn parse_error(&mut self) {
        self.counts.parse_errors += 1;
    }
}

/// One source's scan: the sink, the path-hash salt and the counters.
pub struct ScanCx<'a> {
    out: &'a Sink,
    salt: [u64; 2],
    pub counts: Counts,
}

impl<'a> ScanCx<'a> {
    pub fn new(out: &'a Sink, salt: [u64; 2]) -> Self {
        Self {
            out,
            salt,
            counts: Counts::default(),
        }
    }

    pub fn cancelled(&self) -> bool {
        self.out.cancelled()
    }

    /// Read a JSONL log, line by line, calling `f` for each complete line.
    /// Unchanged files are skipped; with `resumable`, a file whose first line
    /// is unchanged and that only grew is read from where it stopped. The
    /// position is saved only when the whole file was read.
    pub fn jsonl(
        &mut self,
        path: &Path,
        resumable: bool,
        mut f: impl FnMut(&str, &mut Emitter<'_>),
    ) {
        self.counts.files += 1;
        let Ok(meta) = std::fs::metadata(path) else {
            self.counts.errors += 1;
            return;
        };
        let (size, mtime) = (meta.len(), mtime_ms(&meta));
        let ph = scan::path_hash(self.salt, path);
        let prev = self.out.file_prev(ph);
        if prev.is_some_and(|p| p.size == size && p.mtime_ms == mtime) {
            self.counts.files_unchanged += 1;
            return;
        }
        let Ok(mut file) = File::open(path) else {
            self.counts.errors += 1;
            return;
        };
        let head = head_hash(&mut file);
        let start = match prev {
            Some(p) if resumable && head != 0 && p.head == head && p.pos > 0 && p.pos <= size => {
                p.pos
            }
            _ => 0,
        };
        if file.seek(SeekFrom::Start(start)).is_err() {
            self.counts.errors += 1;
            return;
        }
        let out = self.out;
        let mut lines = Lines::new(BufReader::with_capacity(64 * 1024, file), start, MAX_LINE);
        let mut n: u64 = 0;
        loop {
            n += 1;
            if n % CANCEL_EVERY == 0 && out.cancelled() {
                return; // progress is not saved for this file: it is read again
            }
            match lines.next_line() {
                Ok(Some(Line::Text(t))) => {
                    self.counts.lines += 1;
                    f(
                        t,
                        &mut Emitter {
                            out,
                            counts: &mut self.counts,
                        },
                    );
                }
                Ok(Some(Line::TooLong)) => self.counts.lines_too_long += 1,
                Ok(Some(Line::Invalid)) => self.counts.lines_invalid += 1,
                Ok(None) => break,
                Err(_) => {
                    self.counts.errors += 1;
                    return;
                }
            }
        }
        self.counts.files_read += 1;
        out.file_done(FileStamp {
            path_hash: ph,
            size,
            mtime_ms: mtime,
            pos: lines.pos(),
            head,
        });
    }

    /// A file that is read whole (a JSON document, a database): skipped when
    /// its size and mtime are unchanged; `f` is told to read it otherwise.
    pub fn whole_file(&mut self, path: &Path, f: impl FnOnce(&mut Emitter<'_>) -> bool) {
        self.counts.files += 1;
        let Ok(meta) = std::fs::metadata(path) else {
            self.counts.errors += 1;
            return;
        };
        let (size, mtime) = (meta.len(), mtime_ms(&meta));
        let ph = scan::path_hash(self.salt, path);
        if self
            .out
            .file_prev(ph)
            .is_some_and(|p| p.size == size && p.mtime_ms == mtime)
        {
            self.counts.files_unchanged += 1;
            return;
        }
        let out = self.out;
        let ok = f(&mut Emitter {
            out,
            counts: &mut self.counts,
        });
        if ok {
            self.counts.files_read += 1;
            out.file_done(FileStamp::new(ph, size, mtime));
        } else {
            self.counts.errors += 1;
        }
    }

    /// A session with no token counts: one request with zero tokens.
    pub fn session_only(&mut self, tool: &'static str, session: &str, time_ms: i64, project: &str) {
        Emitter {
            out: self.out,
            counts: &mut self.counts,
        }
        .request(Request {
            tool,
            id_key: &format!("{session}:session"),
            time_ms,
            session,
            model: "",
            project,
            input: 0,
            output: 0,
            cache_read: 0,
            cache_write: 0,
        });
    }

    /// Read a small log whole, line by line, without remembering anything
    /// about it (an index file read on every scan).
    pub fn plain_lines(&mut self, path: &Path, mut f: impl FnMut(&str)) {
        let Ok(file) = File::open(path) else { return };
        let mut lines = Lines::new(BufReader::new(file), 0, MAX_LINE);
        while let Ok(Some(l)) = lines.next_line() {
            if let Line::Text(t) = l {
                f(t);
            }
        }
    }
}

// ---------- sources: detection, discovery, status ----------

/// A tool whose default folder exists on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub tool: &'static ToolInfo,
    pub root: PathBuf,
    /// Data files found there (0: the tool is installed but has no logs yet).
    pub files: usize,
}

/// Tools whose default folder exists, with how many log files each has.
pub fn detect(env: &Env) -> Vec<Detected> {
    let mut out = Vec::new();
    for a in adapters() {
        for root in a.default_roots(env) {
            if root.is_dir() {
                let files = a.files(&root).len();
                out.push(Detected {
                    tool: a.info(),
                    root,
                    files,
                });
            }
        }
    }
    out
}

fn norm(path: &str) -> String {
    let p = expand_home(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let t = p.trim_end_matches(['/', '\\']);
    if t.is_empty() { p } else { t.to_string() }
}

pub fn new_source_id() -> String {
    format!("{:08x}", scan::new_salt()[0] as u32)
}

pub fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok())
        .collect()
}

/// A new custom source for `tool` at `path` (no duplicates by tool and path).
/// With a bookmark hook, the folder is bookmarked so a move is found again.
pub fn new_custom_source(
    settings: &Settings,
    tool_id: &str,
    path: &str,
    hooks: &ScanHooks,
) -> Result<AiSource, String> {
    tool(tool_id).ok_or_else(|| format!("unknown tool `{tool_id}`"))?;
    let abs = norm(path);
    if !Path::new(&abs).is_absolute() {
        return Err("the folder must be an absolute path".into());
    }
    if !Path::new(&abs).is_dir() {
        return Err("that folder does not exist".into());
    }
    if settings
        .ai_sources
        .iter()
        .any(|s| s.tool == tool_id && norm(&s.path) == abs)
    {
        return Err("that folder is already a source for this tool".into());
    }
    let bookmark = hooks
        .bookmark_create
        .and_then(|f| f(Path::new(&abs)))
        .map(|b| hex_encode(&b));
    Ok(AiSource {
        id: new_source_id(),
        tool: tool_id.to_string(),
        path: abs,
        builtin: false,
        enabled: true,
        bookmark,
    })
}

/// The sources as they should be before a scan, or `None` if nothing changes:
///
/// * a source whose folder is missing but has a bookmark that now points to an
///   existing folder gets the new path quietly (and a refreshed bookmark);
/// * every tool whose default folder exists and is not a source yet is added
///   (once found, a built-in source stays even if the tool is removed).
///
/// Nothing is ever removed here.
pub fn prepare_sources(settings: &Settings, hooks: &ScanHooks, env: &Env) -> Option<Vec<AiSource>> {
    let mut sources = settings.ai_sources.clone();
    if let Some(resolve) = hooks.bookmark_resolve {
        for s in &mut sources {
            if Path::new(&norm(&s.path)).is_dir() {
                continue;
            }
            let Some(bytes) = s.bookmark.as_deref().and_then(hex_decode) else {
                continue;
            };
            if let Some(r) = resolve(&bytes)
                && r.path.is_dir()
            {
                s.path = r.path.to_string_lossy().into_owned();
                if let Some(b) = r.refreshed {
                    s.bookmark = Some(hex_encode(&b));
                }
            }
        }
    }
    for a in adapters() {
        for root in a.default_roots(env) {
            if !root.is_dir() {
                continue;
            }
            let p = root.to_string_lossy().into_owned();
            let id = a.info().id;
            if !sources
                .iter()
                .any(|s| s.tool == id && norm(&s.path) == norm(&p))
            {
                sources.push(AiSource {
                    id: new_source_id(),
                    tool: id.to_string(),
                    path: p,
                    builtin: true,
                    enabled: true,
                    bookmark: None,
                });
            }
        }
    }
    (sources != settings.ai_sources).then_some(sources)
}

/// What the last scan saw of one source.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceStatus {
    /// `ok`, `missing` or `error`.
    pub state: String,
    pub last_seen_ms: Option<i64>,
    pub last_scan_ms: Option<i64>,
    pub files: u64,
    pub requests: u64,
}

#[derive(Default)]
struct State {
    salt: Option<[u64; 2]>,
    sources: BTreeMap<String, SourceStatus>,
}

impl State {
    fn parse(s: Option<&str>) -> Self {
        let Some(v) = s.and_then(|s| serde_json::from_str::<Value>(s).ok()) else {
            return Self::default();
        };
        let salt = v["salt"].as_array().and_then(|a| {
            let a: Vec<u64> = a.iter().filter_map(|x| x.as_str()?.parse().ok()).collect();
            (a.len() == 2).then(|| [a[0], a[1]])
        });
        Self {
            salt,
            sources: parse_status_value(&v),
        }
    }
    fn to_json(&self) -> String {
        let sources: serde_json::Map<String, Value> = self
            .sources
            .iter()
            .map(|(id, s)| {
                (
                    id.clone(),
                    json!({
                        "state": s.state,
                        "last_seen_ms": s.last_seen_ms,
                        "last_scan_ms": s.last_scan_ms,
                        "files": s.files,
                        "requests": s.requests,
                    }),
                )
            })
            .collect();
        json!({
            "v": 1,
            "salt": self.salt.map(|s| [s[0].to_string(), s[1].to_string()]),
            "sources": sources,
        })
        .to_string()
    }
}

fn parse_status_value(v: &Value) -> BTreeMap<String, SourceStatus> {
    v["sources"]
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(id, s)| {
                    (
                        id.clone(),
                        SourceStatus {
                            state: s["state"].as_str().unwrap_or("").to_string(),
                            last_seen_ms: s["last_seen_ms"].as_i64(),
                            last_scan_ms: s["last_scan_ms"].as_i64(),
                            files: uint(&s["files"]),
                            requests: uint(&s["requests"]),
                        },
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Per-source status from a stored `scan_state.state` of the AI collector.
pub fn parse_status(state_json: &str) -> BTreeMap<String, SourceStatus> {
    serde_json::from_str::<Value>(state_json)
        .map(|v| parse_status_value(&v))
        .unwrap_or_default()
}

/// Scan every enabled source in `out.settings()`. Runs on the scan thread.
pub fn scan_sources(out: &Sink) {
    let settings = out.settings();
    let mut state = State::parse(out.scan_state());
    let salt = *state.salt.get_or_insert_with(scan::new_salt);
    let now = out.now().utc_ms;
    for src in &settings.ai_sources {
        if out.cancelled() {
            return;
        }
        let Some(adapter) = adapter(&src.tool) else {
            continue;
        };
        if !src.enabled {
            continue;
        }
        out.count("sources", 1);
        let st = state.sources.entry(src.id.clone()).or_default();
        let root = PathBuf::from(norm(&src.path));
        if !root.is_dir() {
            // Kept, and checked again next time.
            st.state = "missing".into();
            st.last_scan_ms = Some(now);
            out.count("sources_missing", 1);
            out.save_scan_state(state.to_json());
            continue;
        }
        let mut cx = ScanCx::new(out, salt);
        adapter.scan(&root, &mut cx);
        if out.cancelled() {
            return; // not marked as scanned: it is read again (cheaply)
        }
        let c = cx.counts;
        st.state = if c.errors > 0 && c.files_read == 0 && c.files_unchanged == 0 {
            "error".into()
        } else {
            "ok".into()
        };
        st.last_seen_ms = Some(now);
        st.last_scan_ms = Some(now);
        st.files = c.files;
        st.requests = c.requests;
        out.count("files", c.files);
        out.count("files_read", c.files_read);
        out.count("files_unchanged", c.files_unchanged);
        out.count("lines", c.lines);
        out.count("lines_too_long", c.lines_too_long);
        out.count("parse_errors", c.parse_errors);
        out.count("requests_seen", c.requests);
        out.count("errors", c.errors);
        out.save_scan_state(state.to_json());
    }
}

/// The OS string of an env var as a path list split on `,` (Codex).
pub fn split_roots(v: OsString) -> Vec<PathBuf> {
    v.to_string_lossy()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_name_is_only_the_last_component() {
        assert_eq!(folder_name("/Users/me/code/wrapped"), "wrapped");
        assert_eq!(folder_name("/Users/me/code/wrapped/"), "wrapped");
        assert_eq!(folder_name("C:\\Users\\me\\proj"), "proj");
        assert_eq!(folder_name("file:///home/me/x"), "x");
        assert_eq!(folder_name(""), "");
        assert_eq!(folder_name("/"), "");
    }

    #[test]
    fn hex_round_trip() {
        let b = vec![0u8, 1, 0xab, 0xff];
        assert_eq!(hex_decode(&hex_encode(&b)).unwrap(), b);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
    }

    #[test]
    fn request_ids_differ_by_tool_and_key() {
        assert_ne!(request_id("claude", "a"), request_id("codex", "a"));
        assert_ne!(request_id("claude", "a"), request_id("claude", "b"));
        assert_eq!(request_id("claude", "a"), request_id("claude", "a"));
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            parse_ts_ms("2026-11-02T01:00:00.500Z"),
            Some(1_793_581_200_500)
        );
        assert_eq!(parse_ts_ms("nope"), None);
    }
}
