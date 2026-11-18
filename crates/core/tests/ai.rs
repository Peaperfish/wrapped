//! The AI usage collector against fixture logs written by the tests (made-up
//! content only). Real engine, real store, real adapters; the home folder is a
//! scratch folder, never the real one.
//!
//! Every fixture puts marker text (`SECRET-...`) and a full path into the log
//! lines where a prompt, response or title would be; `assert_clean` checks
//! that none of it reaches the database.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, types::Value as Sql};
use wrapped_core::ai::{self, Env};
use wrapped_core::collector::{ResolvedBookmark, ScanHooks};
use wrapped_core::collectors::AiUsage;
use wrapped_core::settings::{AiSource, Settings};
use wrapped_core::{Engine, ManualClock, Store, Timestamp};

struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("wrapped-ai-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    fn home(&self) -> PathBuf {
        self.0.join("home")
    }
    fn env(&self) -> Env {
        Env::from_map(self.home(), &[])
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn append(p: &Path, text: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(p).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn clock() -> ManualClock {
    ManualClock::at_local(2026, 11, 20, 9, 0, 0, 480)
}

fn engine(c: &ManualClock, env: Env, sources: Option<&str>) -> Engine {
    let store = Store::open_in_memory().unwrap();
    if let Some(s) = sources {
        store.set_setting("ai.sources", s).unwrap();
    }
    let mut e = Engine::new(
        store,
        Arc::new(c.clone()),
        vec![Box::new(AiUsage::with_env(env))],
    )
    .unwrap();
    e.set_auto_scans(false);
    e
}

/// Scan now and wait; returns the report counters.
fn scan(e: &mut Engine) -> std::collections::BTreeMap<&'static str, u64> {
    assert!(e.start_scan("ai_usage").unwrap());
    let r = e.wait_scans().unwrap();
    assert_eq!(r.len(), 1);
    assert!(r[0].completed);
    r[0].counters.clone()
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Totals {
    input: i64,
    output: i64,
    cache_read: i64,
    cache_write: i64,
    requests: i64,
}

fn totals(e: &Engine) -> Totals {
    e.store()
        .conn()
        .query_row(
            "SELECT coalesce(sum(input),0), coalesce(sum(output),0), coalesce(sum(cache_read),0),
                    coalesce(sum(cache_write),0), coalesce(sum(requests),0) FROM ai_usage",
            [],
            |r| {
                Ok(Totals {
                    input: r.get(0)?,
                    output: r.get(1)?,
                    cache_read: r.get(2)?,
                    cache_write: r.get(3)?,
                    requests: r.get(4)?,
                })
            },
        )
        .unwrap()
}

fn t(input: i64, output: i64, cache_read: i64, cache_write: i64, requests: i64) -> Totals {
    Totals {
        input,
        output,
        cache_read,
        cache_write,
        requests,
    }
}

fn count(e: &Engine, table: &str) -> i64 {
    e.store()
        .conn()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

/// Every cell of every table, as text.
fn dump(conn: &Connection) -> String {
    let mut out = String::new();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for t in tables {
        let mut st = conn.prepare(&format!("SELECT * FROM {t}")).unwrap();
        let n = st.column_count();
        let mut rows = st.query([]).unwrap();
        while let Some(r) = rows.next().unwrap() {
            for i in 0..n {
                match r.get::<_, Sql>(i).unwrap() {
                    Sql::Text(s) => out.push_str(&s),
                    Sql::Integer(v) => out.push_str(&v.to_string()),
                    Sql::Real(v) => out.push_str(&v.to_string()),
                    Sql::Blob(b) => out.push_str(&String::from_utf8_lossy(&b)),
                    Sql::Null => {}
                }
                out.push('|');
            }
            out.push('\n');
        }
    }
    out
}

fn assert_clean(e: &Engine) {
    let d = dump(e.store().conn());
    for needle in ["SECRET", "/Users/x/", "/home/x/"] {
        assert!(!d.contains(needle), "database contains {needle:?}");
    }
}

// ---------- Claude Code ----------

fn claude_line(id: &str, ts: &str, session: &str, model: &str, u: (u64, u64, u64, u64)) -> String {
    format!(
        r#"{{"type":"assistant","timestamp":"{ts}","sessionId":"{session}","cwd":"/Users/x/code/proj","uuid":"u-{id}","message":{{"id":"{id}","model":"{model}","role":"assistant","content":[{{"type":"text","text":"SECRET-RESPONSE-TEXT"}}],"usage":{{"input_tokens":{},"output_tokens":{},"cache_read_input_tokens":{},"cache_creation_input_tokens":{}}}}}}}"#,
        u.0, u.1, u.2, u.3
    ) + "\n"
}

const USER_LINE: &str = r#"{"type":"user","timestamp":"2026-11-18T10:00:00.000Z","cwd":"/Users/x/code/proj","message":{"role":"user","content":"SECRET-PROMPT-TEXT"}}
"#;

fn claude_dir(tmp: &Tmp) -> PathBuf {
    tmp.home().join(".claude").join("projects").join("-Users-x-code-proj")
}

#[test]
fn claude_counts_each_request_once_across_lines_files_resumes_and_rescans() {
    let tmp = Tmp::new("claude");
    let s1 = claude_dir(&tmp).join("s1.jsonl");
    let s2 = claude_dir(&tmp).join("s2.jsonl");
    let m1 = claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (10, 5, 100, 20));
    // The same response again: another content block, 2 s later, same usage.
    let m1b = claude_line("m1", "2026-11-18T10:00:02.000Z", "s1", "model-a", (10, 5, 100, 20));
    let m2 = claude_line("m2", "2026-11-18T10:05:00.000Z", "s1", "model-a", (1, 2, 3, 4));
    let synthetic = claude_line("m3", "2026-11-18T10:06:00.000Z", "s1", "<synthetic>", (0, 0, 0, 0));
    // The tool is in the middle of writing a line.
    let m6 = claude_line("m6", "2026-11-18T10:30:00.000Z", "s1", "model-a", (2, 2, 2, 2));
    let (m6_head, m6_tail) = m6.split_at(40);
    write(
        &s1,
        &format!("{USER_LINE}{m1}{m1b}{m2}{synthetic}{m6_head}"),
    );
    // A forked session: earlier lines copied into a new file, plus one new response.
    let m5 = claude_line("m5", "2026-11-18T11:00:00.000Z", "s2", "model-b", (7, 7, 0, 0));
    write(&s2, &format!("{USER_LINE}{m1}{m2}{m5}"));

    let c = clock();
    let mut e = engine(&c, tmp.env(), None);
    let counters = scan(&mut e);
    // m1 (once), m2, m5. Not the second m1 line, the fork's copies, the
    // synthetic message or the unfinished line.
    assert_eq!(totals(&e), t(18, 14, 103, 24, 3));
    assert_eq!(counters["files"], 2);
    assert_eq!(counters["sources"], 1);
    // The tool was found on its own and stays a source.
    let src = &e.settings().ai_sources;
    assert_eq!(src.len(), 1);
    assert!(src[0].builtin && src[0].tool == "claude");
    // Only the whitelisted fields were kept.
    let project: String = e
        .store()
        .conn()
        .query_row("SELECT DISTINCT project FROM ai_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(project, "proj");
    assert_clean(&e);

    // Nothing changed: no file is opened, nothing is counted.
    let again = scan(&mut e);
    assert_eq!(again["files_unchanged"], 2);
    assert_eq!(again.get("files_read").copied().unwrap_or(0), 0);
    assert_eq!(totals(&e), t(18, 14, 103, 24, 3));

    // The tool finishes the line and writes one more: only those two are read.
    let m4 = claude_line("m4", "2026-11-18T10:40:00.000Z", "s1", "model-a", (4, 4, 0, 0));
    append(&s1, &format!("{m6_tail}{m4}"));
    let grown = scan(&mut e);
    assert_eq!(grown["lines"], 2, "read from the saved position, not from the start");
    assert_eq!(totals(&e), t(24, 20, 105, 26, 5));

    // The log is replaced by a different file (rotated): read again from the
    // start, and what was counted before is not counted twice.
    let n1 = claude_line("n1", "2026-11-19T08:00:00.000Z", "s9", "model-a", (1, 1, 1, 1));
    write(&s1, &format!("{n1}{m1}{m2}"));
    let rotated = scan(&mut e);
    assert_eq!(rotated["files_read"], 1);
    assert_eq!(totals(&e), t(25, 21, 106, 27, 6));

    // The tool deletes its logs: what was imported stays, a rescan adds nothing.
    std::fs::remove_file(&s1).unwrap();
    std::fs::remove_file(&s2).unwrap();
    scan(&mut e);
    assert_eq!(totals(&e), t(25, 21, 106, 27, 6));
    assert_clean(&e);
}

#[test]
fn delete_all_ai_data_then_rescan_recounts_logs_that_still_exist() {
    let tmp = Tmp::new("claude-delete");
    write(
        &claude_dir(&tmp).join("s1.jsonl"),
        &claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (10, 5, 100, 20)),
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), None);
    scan(&mut e);
    assert_eq!(totals(&e), t(10, 5, 100, 20, 1));
    assert_eq!(count(&e, "ai_seen"), 1);
    e.delete_collector_data("ai_usage").unwrap();
    assert_eq!(count(&e, "ai_usage"), 0);
    assert_eq!(count(&e, "ai_seen"), 0);
    assert_eq!(count(&e, "scan_files"), 0);
    // Settings (the source list) stay.
    assert_eq!(e.settings().ai_sources.len(), 1);
    scan(&mut e);
    assert_eq!(totals(&e), t(10, 5, 100, 20, 1));
}

#[test]
fn daily_metrics_are_rolled_up_from_the_buckets() {
    let tmp = Tmp::new("claude-rollup");
    write(
        &claude_dir(&tmp).join("s1.jsonl"),
        &format!(
            "{}{}",
            claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (10, 5, 100, 20)),
            claude_line("m2", "2026-11-18T11:00:00.000Z", "s1", "model-b", (1, 2, 3, 4))
        ),
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), None);
    scan(&mut e);
    let day = Timestamp::new(
        wrapped_core::ai::parse_ts_ms("2026-11-18T10:00:00.000Z").unwrap(),
        ai::local_offset_min(wrapped_core::ai::parse_ts_ms("2026-11-18T10:00:00.000Z").unwrap()),
    )
    .local_date();
    let rows = e.store().daily_metrics(day).unwrap();
    let get = |m: &str, k: &str| {
        rows.iter()
            .find(|r| r.collector == "ai_usage" && r.metric == m && r.key == k)
            .map(|r| r.value)
    };
    assert_eq!(get("tokens", ""), Some(18.0));
    assert_eq!(get("requests", ""), Some(2.0));
    assert_eq!(get("sessions", ""), Some(1.0));
    assert_eq!(get("tokens_by_kind", "cache_read"), Some(103.0));
    assert_eq!(get("tokens_by_tool", "claude"), Some(18.0));
    assert_eq!(get("tokens_by_model", "model-a"), Some(15.0));
    assert_eq!(get("tokens_by_project", "proj"), Some(18.0));
}

// ---------- Antigravity ----------

#[test]
fn antigravity_reads_only_the_full_transcript_and_takes_only_the_folder_name() {
    let tmp = Tmp::new("agy");
    let root = tmp.home().join(".gemini").join("antigravity-cli");
    let conv = "11111111-2222-3333-4444-555555555555";
    write(
        &root.join("history.jsonl"),
        &format!(
            "{{\"conversationId\":\"{conv}\",\"display\":\"SECRET-PROMPT-TEXT\",\"timestamp\":1,\"workspace\":\"/Users/x/code/demo\"}}\n\
             {{\"conversationId\":\"other\",\"display\":\"SECRET\",\"workspace\":\"/Users/x/code/elsewhere\"}}\n"
        ),
    );
    let step = |i: u32, ty: &str, tokens: Option<(u64, u64, u64)>| {
        let tok = tokens.map_or(String::new(), |(a, b, c)| {
            format!(",\"input_tokens\":{a},\"output_tokens\":{b},\"cache_read_tokens\":{c}")
        });
        format!(
            "{{\"step_index\":{i},\"type\":\"{ty}\",\"source\":\"MODEL\",\"status\":\"DONE\",\"created_at\":\"2026-11-02T01:0{i}:00Z\",\"content\":\"SECRET-RESPONSE\"{tok}}}\n"
        )
    };
    let full = format!(
        "{}{}{}{}",
        step(0, "USER_INPUT", None),
        step(1, "PLANNER_RESPONSE", Some((1000, 50, 800))),
        step(2, "PLANNER_RESPONSE", Some((200, 20, 100))),
        step(3, "GENERIC", None),
    );
    let logs = root.join("brain").join(conv).join(".system_generated/logs");
    write(&logs.join("transcript_full.jsonl"), &full);
    // Copies of the same steps that must not be read.
    write(&logs.join("transcript.jsonl"), &full);
    write(&logs.join("chunks/transcript_full/0.jsonl"), &full);

    let c = clock();
    let mut e = engine(&c, tmp.env(), None);
    scan(&mut e);
    assert_eq!(totals(&e), t(1200, 70, 900, 0, 2));
    let (model, project, session): (String, String, String) = e
        .store()
        .conn()
        .query_row("SELECT model, project, session FROM ai_usage", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .unwrap();
    assert_eq!((model.as_str(), project.as_str(), session.as_str()), ("unknown", "demo", conv));
    assert_clean(&e);
    // Again: nothing new.
    scan(&mut e);
    assert_eq!(totals(&e).requests, 2);
}

// ---------- sources: never forgotten, moved, detected ----------

#[test]
fn a_missing_source_is_kept_checked_again_and_resumes_when_it_returns() {
    let tmp = Tmp::new("missing");
    let folder = tmp.0.join("external").join("claude-logs");
    let sources = format!(
        r#"[{{"id":"abc123","tool":"claude","path":"{}","builtin":false,"enabled":true}}]"#,
        folder.display()
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&sources));
    let status = |e: &Engine| {
        let st = e.store().scan_info("ai_usage").unwrap().state.unwrap();
        ai::parse_status(&st).remove("abc123").unwrap()
    };

    let counters = scan(&mut e);
    assert_eq!(counters["sources_missing"], 1);
    let s = status(&e);
    assert_eq!((s.state.as_str(), s.last_seen_ms), ("missing", None));

    // It appears: scanned, remembered.
    write(
        &folder.join("p").join("s.jsonl"),
        &claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (3, 3, 3, 3)),
    );
    c.advance_ms(3_600_000);
    scan(&mut e);
    assert_eq!(totals(&e).requests, 1);
    let s = status(&e);
    assert_eq!(s.state, "ok");
    let seen = s.last_seen_ms.expect("last seen");

    // It disappears (an unmounted volume): "not found, last seen", kept, and
    // nothing already imported is lost.
    std::fs::remove_dir_all(&folder).unwrap();
    c.advance_ms(3_600_000);
    scan(&mut e);
    let s = status(&e);
    assert_eq!(s.state, "missing");
    assert_eq!(s.last_seen_ms, Some(seen));
    assert_eq!(totals(&e).requests, 1);
    assert_eq!(e.settings().ai_sources.len(), 1, "never removed automatically");

    // It comes back with more: the old part is not counted twice.
    write(
        &folder.join("p").join("s.jsonl"),
        &format!(
            "{}{}",
            claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (3, 3, 3, 3)),
            claude_line("m2", "2026-11-18T10:10:00.000Z", "s1", "model-a", (1, 1, 1, 1))
        ),
    );
    c.advance_ms(3_600_000);
    scan(&mut e);
    assert_eq!(totals(&e), t(4, 4, 4, 4, 2));
    assert_eq!(status(&e).state, "ok");
}

#[test]
fn disabled_and_removed_sources() {
    let tmp = Tmp::new("disabled");
    let folder = tmp.0.join("logs");
    write(
        &folder.join("s.jsonl"),
        &claude_line("m1", "2026-11-18T10:00:00.000Z", "s1", "model-a", (3, 3, 3, 3)),
    );
    let sources = format!(
        r#"[{{"id":"abc123","tool":"claude","path":"{}","builtin":false,"enabled":false}}]"#,
        folder.display()
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&sources));
    scan(&mut e);
    assert_eq!(totals(&e).requests, 0, "a disabled source is not read");
    let r = e.handle_request(&wrapped_core::ctl::Request::new(
        "ai_set_source",
        serde_json::json!({"id": "abc123", "enabled": true}),
    ));
    assert!(matches!(r, wrapped_core::ctl::Reply::Ok(_)));
    scan(&mut e);
    assert_eq!(totals(&e).requests, 1);
    // Only an explicit request removes a source.
    let r = e.handle_request(&wrapped_core::ctl::Request::new(
        "ai_remove_source",
        serde_json::json!({"id": "abc123"}),
    ));
    assert!(matches!(r, wrapped_core::ctl::Reply::Ok(_)));
    assert!(e.settings().ai_sources.is_empty());
    assert_eq!(totals(&e).requests, 1, "numbers stay");
}

#[test]
fn tools_on_this_machine_become_sources_once_with_env_overrides() {
    let tmp = Tmp::new("detect");
    let home = tmp.home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    std::fs::create_dir_all(home.join(".gemini/antigravity-cli")).unwrap();
    // A folder that only looks like another tool's must not be taken for it.
    std::fs::create_dir_all(home.join(".gemini")).unwrap();
    let env = Env::from_map(home.clone(), &[]);
    let hooks = ScanHooks::default();
    let settings = Settings::default();
    let found = ai::prepare_sources(&settings, &hooks, &env).expect("two tools found");
    let tools: Vec<&str> = found.iter().map(|s| s.tool.as_str()).collect();
    assert_eq!(tools, ["claude", "antigravity"]);
    assert!(found.iter().all(|s| s.builtin && s.enabled));
    // Once added, found again changes nothing.
    let mut s2 = settings.clone();
    s2.ai_sources = found;
    assert!(ai::prepare_sources(&s2, &hooks, &env).is_none());

    // CLAUDE_CONFIG_DIR moves the folder; the old source stays.
    let moved = tmp.0.join("elsewhere");
    std::fs::create_dir_all(&moved).unwrap();
    let env2 = Env::from_map(
        home.clone(),
        &[("CLAUDE_CONFIG_DIR", moved.to_str().unwrap())],
    );
    let found = ai::prepare_sources(&s2, &hooks, &env2).unwrap();
    assert_eq!(found.iter().filter(|s| s.tool == "claude").count(), 2);

    let d = ai::detect(&env);
    assert_eq!(d.iter().map(|d| d.tool.id).collect::<Vec<_>>(), ["claude", "antigravity"]);
    // Nothing found, nothing added.
    let empty = Env::from_map(tmp.0.join("nope"), &[]);
    assert!(ai::prepare_sources(&settings, &hooks, &empty).is_none());
}

static MOVED_TO: Mutex<Option<PathBuf>> = Mutex::new(None);

fn bookmark_create(p: &Path) -> Option<Vec<u8>> {
    Some(format!("bk:{}", p.display()).into_bytes())
}

fn bookmark_resolve(b: &[u8]) -> Option<ResolvedBookmark> {
    b.starts_with(b"bk:").then(|| ResolvedBookmark {
        path: MOVED_TO.lock().unwrap().clone()?,
        refreshed: Some(b"bk:new".to_vec()),
    })
}

#[test]
fn a_moved_custom_folder_is_found_again_through_its_bookmark() {
    let tmp = Tmp::new("bookmark");
    let old = tmp.0.join("old-name");
    std::fs::create_dir_all(&old).unwrap();
    let hooks = ScanHooks {
        bookmark_create: Some(bookmark_create),
        bookmark_resolve: Some(bookmark_resolve),
        ..Default::default()
    };
    let mut settings = Settings::default();
    let src = ai::new_custom_source(&settings, "codex", old.to_str().unwrap(), &hooks).unwrap();
    assert!(!src.builtin && src.bookmark.is_some());
    // The same folder twice, a missing one and an unknown tool are refused.
    settings.ai_sources.push(src.clone());
    assert!(ai::new_custom_source(&settings, "codex", old.to_str().unwrap(), &hooks).is_err());
    assert!(ai::new_custom_source(&settings, "claude", tmp.0.join("nope").to_str().unwrap(), &hooks).is_err());
    assert!(ai::new_custom_source(&settings, "nonsense", old.to_str().unwrap(), &hooks).is_err());

    // The folder is renamed on the same volume.
    let new = tmp.0.join("new-name");
    std::fs::rename(&old, &new).unwrap();
    *MOVED_TO.lock().unwrap() = Some(new.clone());
    let env = Env::from_map(tmp.0.join("nohome"), &[]);
    let fixed = ai::prepare_sources(&settings, &hooks, &env).expect("path updated");
    assert_eq!(fixed.len(), 1);
    assert_eq!(fixed[0].id, src.id, "same source, new path");
    assert_eq!(fixed[0].path, new.to_string_lossy());
    assert_eq!(fixed[0].bookmark.as_deref(), Some(&*ai::hex_encode(b"bk:new")));

    // Without a bookmark hook (Windows) nothing is guessed.
    assert!(ai::prepare_sources(&settings, &ScanHooks::default(), &env).is_none());
    // A bookmark that points nowhere changes nothing.
    *MOVED_TO.lock().unwrap() = Some(tmp.0.join("gone"));
    assert!(ai::prepare_sources(&settings, &hooks, &env).is_none());
}

// ---------- the other tools (adapters written from documentation: untested on real logs) ----------

fn source_for(tool: &str, root: &Path) -> String {
    format!(
        r#"[{{"id":"src1","tool":"{tool}","path":"{}","builtin":false,"enabled":true}}]"#,
        root.display()
    )
}

#[test]
fn codex_turns_cumulative_totals_into_requests_and_ignores_replayed_history() {
    let tmp = Tmp::new("codex");
    let root = tmp.home().join(".codex");
    let day = root.join("sessions/2026/11/18");
    let meta = |id: &str| {
        format!(
            "{{\"timestamp\":\"2026-11-18T10:00:00.000Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/home/x/work/cx\",\"instructions\":\"SECRET\"}}}}\n\
             {{\"timestamp\":\"2026-11-18T10:00:01.000Z\",\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-test\"}}}}\n\
             {{\"timestamp\":\"2026-11-18T10:00:02.000Z\",\"type\":\"response_item\",\"payload\":{{\"text\":\"SECRET\"}}}}\n"
        )
    };
    let tc = |ts: &str, i: u64, c: u64, o: u64, total: u64| {
        format!(
            "{{\"timestamp\":\"{ts}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"input_tokens\":{i},\"cached_input_tokens\":{c},\"output_tokens\":{o},\"reasoning_output_tokens\":0,\"total_tokens\":{total}}}}}}}}}\n"
        )
    };
    let t1 = tc("2026-11-18T10:01:00.000Z", 100, 40, 10, 110);
    let t3 = tc("2026-11-18T10:03:00.000Z", 250, 100, 30, 280);
    let file1 = day.join("rollout-2026-11-18T10-00-00-aaaa.jsonl");
    write(
        &file1,
        &format!("{}{t1}{t1}{t3}{}", meta("sess-a"), "{\"timestamp\":\"x\",\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":null}}\n"),
    );
    // A fork replays the history under a new session id, then continues.
    let t4 = tc("2026-11-18T10:04:00.000Z", 400, 100, 50, 450);
    write(
        &day.join("rollout-2026-11-18T10-10-00-bbbb.jsonl"),
        &format!("{}{t1}{t3}{t4}", meta("sess-b")),
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&source_for("codex", &root)));
    scan(&mut e);
    // t1: 60 in / 40 cached / 10 out; t3: 90 / 60 / 20; t4: 150 / 0 / 20.
    assert_eq!(totals(&e), t(300, 50, 100, 0, 3));
    let (model, project): (String, String) = e
        .store()
        .conn()
        .query_row("SELECT DISTINCT model, project FROM ai_usage", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((model.as_str(), project.as_str()), ("gpt-test", "cx"));
    assert_clean(&e);
    // The file grows: re-read whole, only the new total counts.
    append(&file1, &tc("2026-11-18T10:09:00.000Z", 500, 150, 60, 560));
    scan(&mut e);
    assert_eq!(totals(&e).requests, 4);
}

#[test]
fn gemini_reads_session_documents_and_jsonl() {
    let tmp = Tmp::new("gemini");
    let root = tmp.home().join(".gemini").join("tmp");
    let proj = root.join("abc123");
    write(&proj.join(".project_root"), "/home/x/work/gproj\n");
    let doc = r#"{"sessionId":"sg1","projectHash":"abc123","startTime":"2026-11-18T09:00:00Z","messages":[
        {"id":"m1","timestamp":"2026-11-18T09:00:01Z","type":"user","content":"SECRET-PROMPT"},
        {"id":"m2","timestamp":"2026-11-18T09:00:05Z","type":"gemini","model":"gemini-test","content":"SECRET-RESPONSE",
         "tokens":{"input":100,"output":20,"cached":30,"thoughts":5,"tool":0,"total":155}},
        {"id":"m3","timestamp":"2026-11-18T09:01:00Z","type":"info","content":"SECRET"}]}"#;
    write(&proj.join("chats/session-1.json"), doc);
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&source_for("gemini", &root)));
    scan(&mut e);
    // input excludes the cached part; thoughts count as output.
    assert_eq!(totals(&e), t(70, 25, 30, 0, 1));
    let (model, project): (String, String) = e
        .store()
        .conn()
        .query_row("SELECT model, project FROM ai_usage", [], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!((model.as_str(), project.as_str()), ("gemini-test", "gproj"));
    assert_clean(&e);
    scan(&mut e);
    assert_eq!(totals(&e).requests, 1);
    // The JSON document is rewritten with one more message: counted once each.
    let doc2 = doc.replace(
        r#"{"id":"m3""#,
        r#"{"id":"m4","timestamp":"2026-11-18T09:02:00Z","type":"gemini","model":"gemini-test","content":"SECRET","tokens":{"input":10,"output":1,"cached":0,"thoughts":0,"tool":0,"total":11}},{"id":"m3""#,
    );
    write(&proj.join("chats/session-1.json"), &doc2);
    scan(&mut e);
    assert_eq!(totals(&e), t(80, 26, 30, 0, 2));
}

#[test]
fn copilot_counts_usage_events_or_falls_back_to_sessions() {
    let tmp = Tmp::new("copilot");
    let root = tmp.home().join(".copilot");
    let ev = |ty: &str, ts: &str, extra: &str| {
        format!("{{\"type\":\"{ty}\",\"id\":\"e-{ts}\",\"timestamp\":\"{ts}\",\"data\":{{\"content\":\"SECRET\"{extra}}}}}\n")
    };
    write(
        &root.join("session-state/sess-1/events.jsonl"),
        &format!(
            "{}{}{}",
            ev("session.start", "2026-11-18T09:00:00Z", ""),
            ev("assistant.usage", "2026-11-18T09:00:05Z", ",\"model\":\"cp-test\",\"inputTokens\":50,\"outputTokens\":5,\"cacheReadTokens\":10,\"cacheWriteTokens\":2"),
            ev("assistant.message", "2026-11-18T09:00:06Z", "")
        ),
    );
    // A session whose usage events were not saved: one request, no tokens.
    write(
        &root.join("session-state/sess-2/events.jsonl"),
        &format!("{}{}", ev("session.start", "2026-11-18T10:00:00Z", ""), ev("assistant.message", "2026-11-18T10:00:06Z", "")),
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&source_for("copilot", &root)));
    scan(&mut e);
    assert_eq!(totals(&e), t(50, 5, 10, 2, 2));
    assert_clean(&e);
    scan(&mut e);
    assert_eq!(totals(&e).requests, 2);
}

#[test]
fn cursor_records_sessions_without_tokens() {
    let tmp = Tmp::new("cursor");
    let root = tmp.home().join(".cursor").join("chats");
    let db = root.join("wshash").join("chat-1").join("store.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT); CREATE TABLE blobs (id TEXT, data BLOB);")
            .unwrap();
        let json = r#"{"agentId":"a1","name":"SECRET-TITLE","createdAt":1793581200000,"lastUsedModel":"cursor-test"}"#;
        conn.execute("INSERT INTO meta VALUES ('0', ?1)", [ai::hex_encode(json.as_bytes())]).unwrap();
        conn.execute("INSERT INTO blobs VALUES ('x', ?1)", [b"SECRET message blob".to_vec()]).unwrap();
    }
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&source_for("cursor", &root)));
    scan(&mut e);
    assert_eq!(totals(&e), t(0, 0, 0, 0, 1));
    let model: String = e
        .store()
        .conn()
        .query_row("SELECT model FROM ai_usage", [], |r| r.get(0))
        .unwrap();
    assert_eq!(model, "cursor-test");
    assert_clean(&e);
    scan(&mut e);
    assert_eq!(totals(&e).requests, 1);
}

#[test]
fn opencode_reads_finished_message_files_and_the_database() {
    let tmp = Tmp::new("opencode");
    let root = tmp.home().join(".local/share/opencode");
    let msg = |id: &str, done: bool, i: u64| {
        let completed = if done { ",\"completed\":1793581300000" } else { "" };
        format!(
            "{{\"id\":\"{id}\",\"sessionID\":\"ses1\",\"role\":\"assistant\",\"modelID\":\"oc-test\",\"time\":{{\"created\":1793581200000{completed}}},\"path\":{{\"cwd\":\"/home/x/work/ocp\"}},\"summary\":\"SECRET\",\"tokens\":{{\"input\":{i},\"output\":4,\"reasoning\":1,\"cache\":{{\"read\":7,\"write\":3}}}}}}"
        )
    };
    write(&root.join("storage/message/ses1/m1.json"), &msg("m1", true, 10));
    write(&root.join("storage/message/ses1/m2.json"), &msg("m2", false, 99)); // still streaming
    write(
        &root.join("storage/message/ses1/u.json"),
        r#"{"id":"u1","role":"user","content":"SECRET"}"#,
    );
    let c = clock();
    let mut e = engine(&c, tmp.env(), Some(&source_for("opencode", &root)));
    scan(&mut e);
    assert_eq!(totals(&e), t(10, 5, 7, 3, 1));
    assert_clean(&e);
    // The streaming message finishes: counted now, once.
    write(&root.join("storage/message/ses1/m2.json"), &msg("m2", true, 20));
    scan(&mut e);
    assert_eq!(totals(&e), t(30, 10, 14, 6, 2));

    // The database form.
    let db = root.join("opencode.db");
    {
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE message (id TEXT, session_id TEXT, data TEXT);").unwrap();
        conn.execute("INSERT INTO message VALUES ('d1', 'ses2', ?1)", [msg("d1", true, 5)]).unwrap();
    }
    scan(&mut e);
    assert_eq!(totals(&e).requests, 3);
    assert_clean(&e);
}

#[test]
fn tool_registry_matches_the_adapters() {
    for t in ai::TOOLS {
        assert!(ai::adapter(t.id).is_some(), "{}", t.id);
        assert!(!t.title.is_empty() && !t.note.is_empty());
    }
    let tested: Vec<&str> = ai::TOOLS.iter().filter(|t| t.tested).map(|t| t.id).collect();
    assert_eq!(tested, ["claude", "antigravity"]);
    assert!(!ai::tool("cursor").unwrap().tokens);
    let _ = AiSource {
        id: String::new(),
        tool: String::new(),
        path: String::new(),
        builtin: false,
        enabled: true,
        bookmark: None,
    };
}
