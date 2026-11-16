//! Scanned collectors end to end: real engine, SQLite, fake clock, fixture
//! git repositories and EXIF files generated here. Needs the `git` command.

use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use wrapped_core::engine::ScanReport;
use wrapped_core::scan::exif::fixture::{self, Shot};
use wrapped_core::scan::exif::{Capped, HARD_CAP, HEAD_WINDOW, MetaError, read_meta};
use wrapped_core::time::MS_PER_HOUR;
use wrapped_core::{Clock, Collector, Engine, ManualClock, Store, SystemEvent, collectors};

// ---------- harness ----------

struct Tmp(PathBuf);
impl Tmp {
    fn new(tag: &str) -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("wrapped-scan-{}-{tag}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn clock() -> ManualClock {
    ManualClock::at_local(2026, 11, 20, 9, 0, 0, 480)
}

fn engine(c: &ManualClock, settings: &[(&str, String)]) -> Engine {
    let store = Store::open_in_memory().unwrap();
    for (k, v) in settings {
        store.set_setting(k, v).unwrap();
    }
    Engine::new(store, Arc::new(c.clone()), collectors::builtin()).unwrap()
}

fn json_list(p: &Path) -> String {
    serde_json::json!([p.to_str().unwrap()]).to_string()
}

fn git_engine(c: &ManualClock, root: &Path) -> Engine {
    engine(
        c,
        &[
            ("collector.git.enabled", "true".into()),
            ("git.folders", json_list(root)),
            ("git.author_emails", r#"["me@example.com"]"#.into()),
        ],
    )
}

fn photo_engine(c: &ManualClock, root: &Path) -> Engine {
    engine(
        c,
        &[
            ("collector.photos.enabled", "true".into()),
            ("photos.folders", json_list(root)),
        ],
    )
}

fn scan(e: &mut Engine, name: &str) -> ScanReport {
    assert!(e.start_scan(name).unwrap());
    let mut r = e.wait_scans().unwrap();
    assert_eq!(r.len(), 1);
    r.remove(0)
}

fn n(r: &ScanReport, k: &str) -> u64 {
    r.counters.get(k).copied().unwrap_or(0)
}

fn metric(e: &Engine, date: &str, collector: &str, m: &str, key: &str) -> Option<f64> {
    e.store()
        .daily_metrics(date.parse().unwrap())
        .unwrap()
        .into_iter()
        .find(|r| r.collector == collector && r.metric == m && r.key == key)
        .map(|r| r.value)
}

fn metric_sum(e: &Engine, collector: &str, m: &str) -> f64 {
    e.store()
        .all_daily_metrics()
        .unwrap()
        .into_iter()
        .filter(|r| r.collector == collector && r.metric == m)
        .map(|r| r.value)
        .sum()
}

fn count(e: &Engine, sql: &str) -> i64 {
    e.store().conn().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn tick(e: &mut Engine) {
    e.handle(SystemEvent::IdleTick {
        secs_since_input: 1.0,
        display_assertion_holders: vec![],
    })
    .unwrap();
}

// ---------- git fixtures ----------

/// An empty file used as the global git config, so fixtures ignore the
/// machine's config (signing, hooks, default branch) on every platform.
fn empty_git_config() -> PathBuf {
    static P: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let p = std::env::temp_dir().join(format!("wrapped-scan-{}-gitconfig", std::process::id()));
        std::fs::write(&p, b"").unwrap();
        p
    })
    .clone()
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", empty_git_config())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git installed");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
}

/// An empty commit by `email` at `date` (RFC 3339 with offset).
fn commit(dir: &Path, email: &str, date: &str, msg: &str) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["commit", "-q", "--allow-empty", "--no-gpg-sign", "-m", msg])
        .env("GIT_CONFIG_GLOBAL", empty_git_config())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A")
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "A")
        .env("GIT_COMMITTER_EMAIL", email)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Repo `alpha` with three commits by me and one by someone else.
fn alpha(root: &Path) -> PathBuf {
    let a = root.join("alpha");
    init_repo(&a);
    // 23:30 at UTC+8 is 15:30Z on the 16th: local day 16th.
    commit(&a, "me@example.com", "2026-11-16T23:30:00+08:00", "1");
    // 22:00 at UTC-5 is 03:00Z on the 17th: local day still the 16th.
    commit(&a, "Me@Example.com", "2026-11-16T22:00:00-05:00", "2");
    // 00:30 at UTC+8 on the 17th is 16:30Z on the 16th: local day 17th.
    commit(&a, "me@example.com", "2026-11-17T00:30:00+08:00", "3");
    commit(&a, "other@example.com", "2026-11-16T10:00:00+08:00", "4");
    a
}

// ---------- git ----------

#[test]
fn git_counts_only_my_commits_on_their_own_local_day() {
    let t = Tmp::new("git1");
    alpha(&t.0);
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    let r = scan(&mut e, "git");
    assert!(r.completed);
    assert_eq!(
        (
            n(&r, "repos"),
            n(&r, "commits_read"),
            n(&r, "commits_matched")
        ),
        (1, 4, 3)
    );
    assert_eq!(metric(&e, "2026-11-16", "git", "commits", ""), Some(2.0));
    assert_eq!(metric(&e, "2026-11-17", "git", "commits", ""), Some(1.0));
    assert_eq!(
        metric(&e, "2026-11-16", "git", "commits_by_repo", "alpha"),
        Some(2.0)
    );
    assert_eq!(
        metric(&e, "2026-11-16", "git", "commits_by_hour", "22"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, "2026-11-16", "git", "commits_by_hour", "23"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, "2026-11-17", "git", "commits_by_hour", "00"),
        Some(1.0)
    );
    // Only time, offset, repo name and hash are stored.
    let attrs: String = e
        .store()
        .conn()
        .query_row("SELECT attrs FROM items LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attrs, r#"{"repo":"alpha"}"#);
    let off: i64 = e
        .store()
        .conn()
        .query_row(
            "SELECT time_off FROM items ORDER BY time_ms DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(off, -300);
}

#[test]
fn git_same_commit_in_two_clones_counts_once() {
    let t = Tmp::new("git2");
    let a = alpha(&t.0);
    git(&t.0, &["clone", "-q", a.to_str().unwrap(), "beta"]);
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    let r = scan(&mut e, "git");
    assert_eq!(n(&r, "repos"), 2);
    assert_eq!(n(&r, "commits_matched"), 6, "read in both clones");
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 3, "stored once");
    assert_eq!(metric_sum(&e, "git", "commits"), 3.0);
}

/// Run git with a fixed committer (rebase / amend / cherry-pick need one).
/// The author is never set here, so rewriting keeps the original author.
fn git_as_committer(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", empty_git_config())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_COMMITTER_NAME", "C")
        .env("GIT_COMMITTER_EMAIL", "c@example.com")
        .env("GIT_COMMITTER_DATE", "2026-11-19T12:00:00+00:00")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn rev(dir: &Path, r: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", r])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn my_items(e: &Engine) -> i64 {
    count(e, "SELECT count(*) FROM items WHERE collector='git'")
}

#[test]
fn git_force_rebase_onto_moved_base_counts_once() {
    let t = Tmp::new("rebase");
    let a = t.0.join("repo");
    init_repo(&a);
    commit(&a, "other@example.com", "2026-11-15T09:00:00+08:00", "base");
    git(&a, &["checkout", "-q", "-b", "feature"]);
    for (i, h) in [10, 11, 12].iter().enumerate() {
        commit(
            &a,
            "me@example.com",
            &format!("2026-11-16T{h}:00:00+08:00"),
            &format!("f{i}"),
        );
    }
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    scan(&mut e, "git");
    assert_eq!(my_items(&e), 3);
    let before = rev(&a, "feature");
    git(&a, &["checkout", "-q", "main"]);
    commit(
        &a,
        "other@example.com",
        "2026-11-17T09:00:00+08:00",
        "moved base",
    );
    git(&a, &["checkout", "-q", "feature"]);
    git_as_committer(
        &a,
        &["rebase", "-q", "main", "--force-rebase", "-x", "true"],
    );
    assert_ne!(rev(&a, "feature"), before, "new hashes");
    // Keep the old commits reachable too, as a backup branch would.
    git(&a, &["branch", "old-feature", &before]);
    let r = scan(&mut e, "git");
    assert_eq!(
        n(&r, "commits_matched"),
        3,
        "the rebased commits are read as new"
    );
    assert_eq!(my_items(&e), 3);
    assert_eq!(metric_sum(&e, "git", "commits"), 3.0);
}

#[test]
fn git_amend_without_edit_counts_once() {
    let t = Tmp::new("amend");
    let a = t.0.join("repo");
    init_repo(&a);
    commit(&a, "me@example.com", "2026-11-16T10:00:00+08:00", "work");
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    scan(&mut e, "git");
    std::fs::write(a.join("f.txt"), "x").unwrap();
    git(&a, &["add", "f.txt"]);
    let before = rev(&a, "HEAD");
    git_as_committer(
        &a,
        &["commit", "-q", "--amend", "--no-edit", "--no-gpg-sign"],
    );
    assert_ne!(rev(&a, "HEAD"), before);
    scan(&mut e, "git");
    assert_eq!(my_items(&e), 1);
}

#[test]
fn git_cherry_pick_counts_once() {
    let t = Tmp::new("pick");
    let a = t.0.join("repo");
    init_repo(&a);
    commit(&a, "other@example.com", "2026-11-15T09:00:00+08:00", "base");
    git(&a, &["branch", "release"]);
    commit(&a, "me@example.com", "2026-11-16T10:00:00+08:00", "fix");
    let fix = rev(&a, "HEAD");
    git(&a, &["checkout", "-q", "release"]);
    git_as_committer(&a, &["cherry-pick", "--allow-empty", &fix]);
    assert_ne!(rev(&a, "release"), fix);
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    let r = scan(&mut e, "git");
    assert_eq!(n(&r, "commits_matched"), 2);
    assert_eq!(my_items(&e), 1);
}

#[test]
fn git_two_commits_in_the_same_second_count_twice() {
    let t = Tmp::new("same-sec");
    let a = t.0.join("repo");
    init_repo(&a);
    commit(&a, "me@example.com", "2026-11-16T10:00:00+08:00", "one");
    commit(&a, "me@example.com", "2026-11-16T10:00:00+08:00", "two");
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    scan(&mut e, "git");
    assert_eq!(my_items(&e), 2);
}

#[test]
fn git_incremental_scan_reads_only_new_commits() {
    let t = Tmp::new("git3");
    let a = alpha(&t.0);
    let b = t.0.join("nested/gamma");
    init_repo(&b);
    commit(&b, "me@example.com", "2026-11-18T09:00:00+08:00", "g1");
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    let r1 = scan(&mut e, "git");
    assert_eq!((n(&r1, "repos"), n(&r1, "commits_read")), (2, 5));

    commit(&a, "me@example.com", "2026-11-19T09:00:00+08:00", "5");
    commit(&a, "other@example.com", "2026-11-19T10:00:00+08:00", "6");
    let r2 = scan(&mut e, "git");
    assert_eq!(n(&r2, "commits_read"), 2, "only the two new commits");
    assert_eq!(n(&r2, "commits_matched"), 1);
    assert_eq!(n(&r2, "repos_unchanged"), 1);
    assert_eq!(metric(&e, "2026-11-19", "git", "commits", ""), Some(1.0));

    let r3 = scan(&mut e, "git");
    assert_eq!(n(&r3, "commits_read"), 0);
    assert_eq!(n(&r3, "repos_unchanged"), 2);
    assert_eq!(metric_sum(&e, "git", "commits"), 5.0);
}

#[test]
fn git_cancel_then_resume_matches_one_full_scan() {
    let t = Tmp::new("git4");
    alpha(&t.0);
    for name in ["r1", "r2", "r3"] {
        let d = t.0.join(name);
        init_repo(&d);
        for h in 10..14 {
            commit(
                &d,
                "me@example.com",
                &format!("2026-11-1{}T{h}:00:00+01:00", &name[1..]),
                "x",
            );
        }
    }
    let c = clock();
    let mut full = git_engine(&c, &t.0);
    scan(&mut full, "git");
    let want = metric_sum(&full, "git", "commits");
    assert_eq!(want, 15.0);

    let mut e = git_engine(&c, &t.0);
    e.set_scan_cancel_after(Some(4));
    let r = scan(&mut e, "git");
    assert!(!r.completed);
    let partial = count(&e, "SELECT count(*) FROM items");
    assert!(partial < 15, "stopped part-way ({partial})");
    e.set_scan_cancel_after(None);
    let r = scan(&mut e, "git");
    assert!(r.completed);
    assert_eq!(metric_sum(&e, "git", "commits"), want);
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 15);
    assert_eq!(
        e.store().all_daily_metrics().unwrap(),
        full.store().all_daily_metrics().unwrap()
    );
}

// ---------- photos ----------

fn a7(taken: &'static str) -> Shot {
    Shot {
        make: Some("SONY"),
        model: Some("ILCE-7M4"),
        taken: Some(taken),
        subsec: Some("42"),
        offset: Some("+08:00"),
        lens: Some("FE 35mm F1.8"),
        focal: Some((350, 10)),
        f_number: Some((18, 10)),
        exposure: Some((1, 250)),
        iso: Some(400),
        ..Default::default()
    }
}

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

#[test]
fn photos_raw_jpeg_pair_counts_once_and_metrics() {
    let t = Tmp::new("ph1");
    let s = a7("2026:11:14 23:30:00");
    write(&t.0.join("DSC00001.ARW"), &fixture::tiff(&s, None));
    write(&t.0.join("DSC00001.JPG"), &fixture::jpeg(&s));
    let phone = Shot {
        make: Some("Apple"),
        model: Some("iPhone 15 Pro"),
        taken: Some("2026:11:14 08:00:00"),
        lens: Some("iPhone 15 Pro back camera 6.765mm f/1.78"),
        focal: Some((6765, 1000)),
        ..Default::default()
    };
    write(
        &t.0.join("sub/IMG_0001.heic"),
        &fixture::heic(&phone, 50_000),
    );
    write(&t.0.join("notes.txt"), b"not a photo");
    let c = clock();
    let mut e = photo_engine(&c, &t.0);
    let r = scan(&mut e, "photos");
    assert!(r.completed);
    assert_eq!((n(&r, "files_seen"), n(&r, "parsed")), (3, 3));
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 2);
    let d = "2026-11-14";
    assert_eq!(metric(&e, d, "photos", "shots", ""), Some(2.0));
    assert_eq!(
        metric(&e, d, "photos", "shots_by_camera", "SONY ILCE-7M4"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, d, "photos", "shots_by_camera", "Apple iPhone 15 Pro"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, d, "photos", "shots_by_lens", "FE 35mm F1.8"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, d, "photos", "shots_by_focal_length", "35"),
        Some(1.0)
    );
    assert_eq!(
        metric(&e, d, "photos", "shots_by_focal_length", "7"),
        Some(1.0)
    );
    assert_eq!(metric(&e, d, "photos", "shots_by_hour", "23"), Some(1.0));
    assert_eq!(metric(&e, d, "photos", "shots_by_hour", "08"), Some(1.0));
    // The phone shot had no offset: camera time as written, offset unknown.
    assert_eq!(
        count(&e, "SELECT count(*) FROM items WHERE time_off IS NULL"),
        1
    );
    // No path or file name anywhere in the database.
    for sql in [
        "SELECT group_concat(attrs) FROM items",
        "SELECT group_concat(state) FROM scan_state",
    ] {
        let all: String = e.store().conn().query_row(sql, [], |r| r.get(0)).unwrap();
        assert!(
            !all.contains("DSC") && !all.contains("IMG") && !all.contains("wrapped-scan"),
            "{all}"
        );
    }
}

#[test]
fn photos_moved_file_not_double_counted_and_unchanged_not_reread() {
    let t = Tmp::new("ph2");
    write(
        &t.0.join("a/one.jpg"),
        &fixture::jpeg(&a7("2026:11:14 10:00:00")),
    );
    write(
        &t.0.join("a/two.jpg"),
        &fixture::jpeg(&a7("2026:11:14 11:00:00")),
    );
    let c = clock();
    let mut e = photo_engine(&c, &t.0);
    let r1 = scan(&mut e, "photos");
    assert_eq!(n(&r1, "parsed"), 2);
    assert!(n(&r1, "bytes_read") > 0);

    // Unchanged: nothing opened, nothing read, nothing written.
    let r2 = scan(&mut e, "photos");
    assert_eq!(
        (
            n(&r2, "files_unchanged"),
            n(&r2, "parsed"),
            n(&r2, "bytes_read")
        ),
        (2, 0, 0)
    );

    // Move + rename: read again (new path), but the same shot.
    std::fs::create_dir_all(t.0.join("b")).unwrap();
    std::fs::rename(t.0.join("a/one.jpg"), t.0.join("b/renamed.JPG")).unwrap();
    let r3 = scan(&mut e, "photos");
    assert_eq!((n(&r3, "files_unchanged"), n(&r3, "parsed")), (1, 1));
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 2);
    assert_eq!(metric(&e, "2026-11-14", "photos", "shots", ""), Some(2.0));
    // The old path's cache entry is gone; one entry per file.
    assert_eq!(count(&e, "SELECT count(*) FROM scan_files"), 2);
}

#[test]
fn photos_file_without_date_is_skipped_and_counted() {
    let t = Tmp::new("ph3");
    let undated = Shot {
        taken: None,
        ..a7("x")
    };
    write(&t.0.join("scan.jpg"), &fixture::jpeg(&undated));
    write(&t.0.join("broken.jpg"), b"\xFF\xD8\xFF\xDA\0\x02");
    write(
        &t.0.join("ok.jpg"),
        &fixture::jpeg(&a7("2026:11:14 10:00:00")),
    );
    let c = clock();
    let mut e = photo_engine(&c, &t.0);
    let r = scan(&mut e, "photos");
    assert_eq!(n(&r, "files_seen"), 3);
    assert_eq!(n(&r, "skipped_no_date"), 1);
    assert_eq!(n(&r, "skipped_no_exif"), 1);
    assert_eq!(n(&r, "parsed"), 1);
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 1);
    // Skipped files are cached too: not re-read next time.
    let r = scan(&mut e, "photos");
    assert_eq!(n(&r, "files_unchanged"), 3);
}

#[test]
fn photos_cancel_then_resume_matches_one_full_scan() {
    let t = Tmp::new("ph4");
    for i in 0..40 {
        let dir = format!("d{}", i % 4);
        let taken = leak(format!(
            "2026:11:{:02} {:02}:{:02}:00",
            10 + i % 5,
            i % 24,
            i
        ));
        write(
            &t.0.join(dir).join(format!("{i}.jpg")),
            &fixture::jpeg(&a7(taken)),
        );
    }
    let c = clock();
    let mut full = photo_engine(&c, &t.0);
    scan(&mut full, "photos");
    assert_eq!(metric_sum(&full, "photos", "shots"), 40.0);

    let mut e = photo_engine(&c, &t.0);
    e.set_scan_cancel_after(Some(15));
    let r = scan(&mut e, "photos");
    assert!(!r.completed);
    let done = n(&r, "parsed");
    assert!(done > 0 && done < 40, "{done}");
    e.set_scan_cancel_after(None);
    let r = scan(&mut e, "photos");
    assert!(r.completed);
    assert_eq!(n(&r, "files_unchanged"), done, "resumed, not restarted");
    assert_eq!(n(&r, "parsed"), 40 - done);
    assert_eq!(
        e.store().all_daily_metrics().unwrap(),
        full.store().all_daily_metrics().unwrap()
    );
}

#[test]
fn photos_large_scan_completes_under_backpressure() {
    let t = Tmp::new("ph5");
    let n_files = wrapped_core::collector::SCAN_BLOCK_AT + 700;
    let bytes: Vec<Vec<u8>> = (0..n_files)
        .map(|i| {
            let taken = leak(format!(
                "2026:11:{:02} {:02}:{:02}:{:02}",
                1 + i / 3600 % 28,
                i / 60 % 24,
                i % 60,
                i / 1440 % 60
            ));
            fixture::jpeg(&a7(taken))
        })
        .collect();
    for (i, b) in bytes.iter().enumerate() {
        write(&t.0.join(format!("{}/{i}.jpg", i / 500)), b);
    }
    let c = clock();
    let mut e = photo_engine(&c, &t.0);
    let r = scan(&mut e, "photos");
    assert!(r.completed);
    assert_eq!(n(&r, "parsed"), n_files as u64);
    assert_eq!(count(&e, "SELECT count(*) FROM items") as usize, n_files);
}

// ---------- read cap ----------

/// A 60 MB "raw" served lazily: `data` then zeros. Fails the test if more than
/// `limit` bytes are ever read from it.
struct BigFile {
    data: Vec<u8>,
    len: u64,
    pos: u64,
    total: u64,
    limit: u64,
}

impl Read for BigFile {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min((self.len - self.pos.min(self.len)) as usize);
        for (i, b) in buf[..n].iter_mut().enumerate() {
            *b = self.data.get((self.pos as usize) + i).copied().unwrap_or(0);
        }
        self.pos += n as u64;
        self.total += n as u64;
        assert!(
            self.total <= self.limit,
            "read {} bytes, cap {}",
            self.total,
            self.limit
        );
        Ok(n)
    }
}

impl Seek for BigFile {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.pos = match p {
            SeekFrom::Start(o) => o,
            SeekFrom::End(o) => (self.len as i64 + o) as u64,
            SeekFrom::Current(o) => (self.pos as i64 + o) as u64,
        };
        Ok(self.pos)
    }
}

fn big(data: Vec<u8>, limit: u64) -> BigFile {
    BigFile {
        data,
        len: 60 * 1024 * 1024,
        pos: 0,
        total: 0,
        limit,
    }
}

#[test]
fn read_cap_is_respected_for_large_raws() {
    let s = a7("2026:11:14 10:00:00");
    // Exif IFD near the start: only the head window is read.
    let mut f = Capped::new(big(fixture::tiff(&s, None), HEAD_WINDOW), HARD_CAP);
    let m = read_meta(&mut f, 60 * 1024 * 1024).unwrap();
    assert!(m.taken.is_some());
    assert!(f.read <= HEAD_WINDOW);

    // Typical ARW layout (Exif IFD ~5 KB in): nothing beyond the 64 KB window.
    let mut f = Capped::new(big(fixture::tiff(&s, Some(4606)), 64 * 1024), HARD_CAP);
    let m = read_meta(&mut f, 127 * 1024 * 1024).unwrap();
    assert_eq!(m.lens.as_deref(), Some("FE 35mm F1.8"));
    assert!(f.read <= 64 * 1024, "read {}", f.read);

    // Exif IFD straddling the end of the window: extended by a few hundred bytes.
    let mut f = Capped::new(big(fixture::tiff(&s, Some(65_500)), HARD_CAP), HARD_CAP);
    let m = read_meta(&mut f, 60 * 1024 * 1024).unwrap();
    assert_eq!(m.iso, Some(400));
    assert_eq!(m.lens.as_deref(), Some("FE 35mm F1.8"));
    assert!(f.read <= 66 * 1024, "read {}", f.read);

    // Exif IFD at 1 MB: the window is extended to it, still under the cap.
    let mut f = Capped::new(big(fixture::tiff(&s, Some(1 << 20)), HARD_CAP), HARD_CAP);
    let m = read_meta(&mut f, 60 * 1024 * 1024).unwrap();
    assert!(m.taken.is_some(), "found after extending");
    assert_eq!(m.lens.as_deref(), Some("FE 35mm F1.8"));
    assert!(
        f.read <= (1 << 20) + 4096,
        "only up to the IFD and its values"
    );

    // Exif IFD beyond the hard cap: given up on, never read past the cap.
    let mut f = Capped::new(big(fixture::tiff(&s, Some(3 << 20)), HARD_CAP), HARD_CAP);
    match read_meta(&mut f, 60 * 1024 * 1024) {
        Ok(m) => assert!(m.taken.is_none()),
        Err(e) => assert!(matches!(e, MetaError::NoExif), "{e:?}"),
    }
    assert!(f.read <= HARD_CAP);
}

// ---------- scheduling ----------

#[test]
fn daily_scan_never_twice_in_24h_and_not_while_paused() {
    let t = Tmp::new("sched");
    let c = clock();
    let mut e = git_engine(&c, &t.0);
    let t0 = c.now().utc_ms;
    tick(&mut e); // never scanned: starts now
    assert_eq!(e.wait_scans().unwrap().len(), 1);
    assert_eq!(e.last_scan_ms("git").unwrap(), Some(t0));

    c.advance_ms(23 * MS_PER_HOUR + 59 * 60_000);
    tick(&mut e);
    assert!(e.scanning().is_empty());
    assert!(e.wait_scans().unwrap().is_empty(), "not again within 24 h");

    c.advance_ms(60_000);
    tick(&mut e);
    assert_eq!(e.wait_scans().unwrap().len(), 1, "due after 24 h");
    let t1 = c.now().utc_ms;
    assert_eq!(e.last_scan_ms("git").unwrap(), Some(t1));

    e.set_pause(wrapped_core::Pause::Indefinite).unwrap();
    c.advance_ms(30 * MS_PER_HOUR);
    tick(&mut e);
    assert!(e.wait_scans().unwrap().is_empty(), "not while paused");
    assert!(e.start_scan("git").is_err(), "not even on demand");

    e.set_pause(wrapped_core::Pause::Off).unwrap();
    tick(&mut e);
    assert_eq!(e.wait_scans().unwrap().len(), 1, "after resume");

    // Photos is off by default: never scanned.
    assert_eq!(e.last_scan_ms("photos").unwrap(), None);
}

#[test]
fn cancelled_scan_resumes_on_next_tick() {
    let t = Tmp::new("resume");
    write(
        &t.0.join("1.jpg"),
        &fixture::jpeg(&a7("2026:11:14 10:00:00")),
    );
    write(
        &t.0.join("2.jpg"),
        &fixture::jpeg(&a7("2026:11:14 11:00:00")),
    );
    let c = clock();
    let mut e = photo_engine(&c, &t.0);
    e.set_scan_cancel_after(Some(2));
    tick(&mut e);
    let r = e.wait_scans().unwrap();
    assert!(!r[0].completed);
    assert_eq!(
        e.last_scan_ms("photos").unwrap(),
        None,
        "incomplete scans do not count"
    );
    e.set_scan_cancel_after(None);
    c.advance_ms(60_000);
    tick(&mut e);
    let r = e.wait_scans().unwrap();
    assert!(r[0].completed);
    assert_eq!(metric_sum(&e, "photos", "shots"), 2.0);
}

// ---------- delete ----------

#[test]
fn delete_clears_items_scan_state_and_file_cache() {
    let t = Tmp::new("del");
    write(
        &t.0.join("1.jpg"),
        &fixture::jpeg(&a7("2026:11:14 10:00:00")),
    );
    let g = t.0.join("code");
    std::fs::create_dir_all(&g).unwrap();
    alpha(&g);
    let c = clock();
    let mut e = engine(
        &c,
        &[
            ("collector.photos.enabled", "true".into()),
            ("photos.folders", json_list(&t.0)),
            ("collector.git.enabled", "true".into()),
            ("git.folders", json_list(&g)),
            ("git.author_emails", r#"["me@example.com"]"#.into()),
        ],
    );
    scan(&mut e, "photos");
    scan(&mut e, "git");
    assert_eq!(
        count(&e, "SELECT count(*) FROM items WHERE collector='photos'"),
        1
    );
    assert_eq!(count(&e, "SELECT count(*) FROM scan_files"), 1);

    e.delete_collector_data("photos").unwrap();
    for sql in [
        "SELECT count(*) FROM items WHERE collector='photos'",
        "SELECT count(*) FROM scan_state WHERE collector='photos'",
        "SELECT count(*) FROM scan_files WHERE collector='photos'",
        "SELECT count(*) FROM daily_metrics WHERE collector='photos'",
    ] {
        assert_eq!(count(&e, sql), 0, "{sql}");
    }
    assert_eq!(e.last_scan_ms("photos").unwrap(), None);
    // Git untouched.
    assert_eq!(
        count(&e, "SELECT count(*) FROM items WHERE collector='git'"),
        3
    );

    // Turning off with "delete" does the same, and the collector stays off.
    e.disable_collector("git", false).unwrap();
    assert_eq!(count(&e, "SELECT count(*) FROM items"), 0);
    assert_eq!(count(&e, "SELECT count(*) FROM scan_state"), 0);
    assert!(e.start_scan("git").is_err());
}

#[test]
fn git_email_report_aggregates_across_repos_and_clones() {
    let t = Tmp::new("emails");
    let a = alpha(&t.0);
    git(&t.0, &["clone", "-q", a.to_str().unwrap(), "beta"]);
    let g = t.0.join("gamma");
    init_repo(&g);
    commit(
        &g,
        "old@example.com",
        "2024-01-01T09:00:00+00:00",
        "old work",
    );
    commit(
        &g,
        "me@example.com",
        "2026-11-18T09:00:00+08:00",
        "new work",
    );
    let r = wrapped_core::collectors::git::author_email_report(&[t.0.to_str().unwrap().into()]);
    let got: Vec<(String, usize, Vec<String>)> = r
        .into_iter()
        .map(|s| (s.email, s.commits, s.repos.into_iter().collect()))
        .collect();
    let v = |x: &[&str]| x.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        got,
        vec![
            // Mixed-case "Me@Example.com" merges in; clone copies count once.
            ("me@example.com".into(), 4, v(&["alpha", "beta", "gamma"])),
            ("old@example.com".into(), 1, v(&["gamma"])),
            ("other@example.com".into(), 1, v(&["alpha", "beta"])),
        ]
    );
}

fn set_emails(e: &mut Engine, json: &str) {
    e.store().set_setting("git.author_emails", json).unwrap();
    tick(e); // the running collector picks it up (and rescans if needed)
    e.wait_scans().unwrap();
}

#[test]
fn changing_author_emails_rescans_history() {
    let t = Tmp::new("emails-change");
    let r = t.0.join("repo");
    init_repo(&r);
    commit(&r, "b@example.com", "2025-03-01T09:00:00+00:00", "old b 1");
    commit(&r, "b@example.com", "2025-03-02T09:00:00+00:00", "old b 2");
    commit(&r, "me@example.com", "2026-11-16T09:00:00+00:00", "a");
    let c = clock();
    let mut e = git_engine(&c, &t.0); // [me@example.com]
    scan(&mut e, "git");
    assert_eq!(metric_sum(&e, "git", "commits"), 1.0);
    assert_eq!(metric(&e, "2025-03-01", "git", "commits", ""), None);

    // (a) add B: the older commits under B are now counted.
    c.advance_ms(60_000);
    set_emails(&mut e, r#"["me@example.com", "b@example.com"]"#);
    assert_eq!(metric_sum(&e, "git", "commits"), 3.0);
    assert_eq!(metric(&e, "2025-03-01", "git", "commits", ""), Some(1.0));
    assert_eq!(my_items(&e), 3);

    // (b) remove B again: its commits leave items and daily_metrics.
    c.advance_ms(60_000);
    set_emails(&mut e, r#"["me@example.com"]"#);
    assert_eq!(my_items(&e), 1);
    assert_eq!(metric_sum(&e, "git", "commits"), 1.0);
    assert_eq!(metric(&e, "2025-03-01", "git", "commits", ""), None);

    // (c) the same list again (any order / case): no reset, no rescan.
    let last = e.last_scan_ms("git").unwrap();
    c.advance_ms(60_000);
    set_emails(&mut e, r#"["ME@example.com"]"#);
    assert_eq!(e.last_scan_ms("git").unwrap(), last);
    assert_eq!(my_items(&e), 1);
}

#[test]
fn email_change_made_offline_resets_on_next_start() {
    let t = Tmp::new("emails-offline");
    let r = t.0.join("repo");
    init_repo(&r);
    commit(&r, "b@example.com", "2025-03-01T09:00:00+00:00", "old b");
    commit(&r, "me@example.com", "2026-11-16T09:00:00+00:00", "a");
    let path = t.0.join("w.db");
    let c = clock();
    let settings = |s: &Store| {
        s.set_setting("collector.git.enabled", "true").unwrap();
        s.set_setting("git.folders", &json_list(&t.0)).unwrap();
        s.set_setting("git.author_emails", r#"["me@example.com"]"#)
            .unwrap();
    };
    {
        let s = Store::open(&path).unwrap();
        settings(&s);
        let mut e = Engine::new(s, Arc::new(c.clone()), collectors::builtin()).unwrap();
        scan(&mut e, "git");
        assert_eq!(my_items(&e), 1);
    }
    // Changed while no collector runs (CLI `settings set`): the store resets.
    let mut s = Store::open(&path).unwrap();
    s.set_setting("git.author_emails", r#"["me@example.com","b@example.com"]"#)
        .unwrap();
    let fp = collectors::git::Git
        .scan_fingerprint(&s.load_settings().unwrap().0)
        .unwrap();
    assert!(s.sync_scan_fingerprint("git", &fp).unwrap());
    assert!(
        !s.sync_scan_fingerprint("git", &fp).unwrap(),
        "same list: no reset"
    );
    assert_eq!(s.item_count("git").unwrap(), 0);
    let mut e = Engine::new(s, Arc::new(c.clone()), collectors::builtin()).unwrap();
    scan(&mut e, "git");
    assert_eq!(my_items(&e), 2);
}
