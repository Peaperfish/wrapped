//! Git: commit times and repository (folder) names of your own commits.
//!
//! Records per commit only: author time (UTC + the commit's own author
//! offset, whose local date decides the day), the repository folder name and
//! an id. Never code, diffs, messages, file names, line counts, paths or
//! commit hashes.
//!
//! The id is a hash of (author email, author time in seconds, author offset,
//! first line of the message), not the commit hash, so the same work counts
//! once across clones, rebases (including `--force-rebase`), amends that keep
//! the message, and cherry-picks: all of these keep the author fields. The
//! message line only goes into the hash; it is never stored or logged.
//! Rewording a commit's first line makes it count again.
//!
//! Uses the `git` command line: two short-lived processes per repository per
//! scan (`for-each-ref`, then `log` only if a ref moved), so nothing to compile
//! or link and no process outlives the scan. Incremental: per repository the
//! ref tips seen at the last scan are kept; the next `log` excludes them, so a
//! daily scan reads only new commits.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::collector::{
    Collector, CollectorInfo, DayInput, Kind, MetricDef, MetricValue, Sink, Unit,
};
use crate::scan::{self, Flow, Rules};
use crate::settings::{KEY_GIT_EMAILS, Settings, expand_home};
use crate::time::{MS_PER_SEC, Timestamp};

pub const NAME: &str = "git";

#[derive(Default)]
pub struct Git;

/// Incremental state (`scan_state.state`).
#[derive(Debug, Default)]
struct State {
    salt: Option<[u64; 2]>,
    /// Salted hash of the repository path → ref tips seen at the last scan.
    repos: BTreeMap<String, Vec<String>>,
}

impl State {
    fn parse(s: Option<&str>) -> Self {
        let Some(v) = s.and_then(|s| serde_json::from_str::<Value>(s).ok()) else {
            return Self::default();
        };
        let salt = v["salt"].as_array().and_then(|a| {
            let a: Vec<u64> = a
                .iter()
                .filter_map(|x| x.as_str()?.parse().ok())
                .collect();
            (a.len() == 2).then(|| [a[0], a[1]])
        });
        let repos = v["repos"]
            .as_object()
            .map(|o| {
                o.iter()
                    .map(|(k, tips)| {
                        let tips = tips
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|t| t.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        (k.clone(), tips)
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { salt, repos }
    }
    fn to_json(&self) -> String {
        let salt = self.salt.map(|s| json!([s[0].to_string(), s[1].to_string()]));
        json!({ "salt": salt, "repos": self.repos }).to_string()
    }
}

/// A `git` command with settings that keep it read-only, quiet and short-lived.
fn git(dir: &Path) -> Command {
    let mut c = Command::new("git");
    c.arg("-C")
        .arg(dir)
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "log.showSignature=false",
            "-c",
            "core.quotePath=true",
            "--no-pager",
            "--no-replace-objects",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    c
}

/// `git config --global user.email`, lowercased (empty if unset or no git).
pub fn global_author_email() -> Vec<String> {
    Command::new("git")
        .args(["config", "--global", "user.email"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_lowercase())
        .filter(|e| e.contains('@'))
        .into_iter()
        .collect()
}

/// A repository found by the walk: (work tree or bare dir, display name).
fn repo_at(dir: &Path) -> Option<(PathBuf, String)> {
    let name = dir.file_name()?.to_str()?;
    if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
        return Some((dir.to_path_buf(), name.to_string()));
    }
    // Bare repository: `name.git` with HEAD, objects and refs.
    if let Some(stem) = name.strip_suffix(".git")
        && dir.join("HEAD").is_file()
        && dir.join("objects").is_dir()
        && dir.join("refs").is_dir()
    {
        return Some((dir.to_path_buf(), stem.to_string()));
    }
    None
}

enum RepoOutcome {
    Unchanged,
    Done(Vec<String>),
    Failed,
    Cancelled,
}

/// One commit from `git log`: (id, author time, author email).
///
/// Line format: `<unix secs>\t<YYYY-MM-DD HH:MM:SS +HHMM>\t<email>\t<subject>`
/// (the subject is last, so tabs inside it are harmless).
fn parse_line(line: &str) -> Option<(String, Timestamp, &str)> {
    let mut it = line.splitn(4, '\t');
    let secs_s = it.next()?;
    let secs: i64 = secs_s.parse().ok()?;
    let iso = it.next()?;
    let email = it.next()?;
    let subject = it.next().unwrap_or("");
    let tz = iso.rsplit(' ').next()?;
    let sign = match tz.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m): (i32, i32) = (tz.get(1..3)?.parse().ok()?, tz.get(3..5)?.parse().ok()?);
    let offset = sign * (h * 60 + m);
    Some((
        commit_id(email, secs, offset, subject),
        Timestamp::new(secs * MS_PER_SEC, offset),
        email,
    ))
}

/// Identity of a piece of work: survives clone, rebase, amend (same message)
/// and cherry-pick, which all keep the author email, time and offset.
fn commit_id(email: &str, secs: i64, offset_min: i32, subject: &str) -> String {
    let key = format!(
        "{}\0{secs}\0{offset_min}\0{}",
        email.to_ascii_lowercase(),
        subject.trim_end()
    );
    scan::hash128_hex(key.as_bytes())
}

fn scan_repo(
    out: &Sink,
    dir: &Path,
    name: &str,
    old: Option<&Vec<String>>,
    emails: &[String],
) -> RepoOutcome {
    let mut cmd = git(dir);
    cmd.args([
        "for-each-ref",
        "--format=%(objectname)",
        "refs/heads",
        "refs/tags",
        "refs/remotes",
    ])
    .stdout(Stdio::piped());
    out.configure_command(&mut cmd);
    let Ok(o) = cmd.output() else {
        return RepoOutcome::Failed;
    };
    if !o.status.success() {
        return RepoOutcome::Failed;
    }
    let tips: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if old == Some(&tips) {
        return RepoOutcome::Unchanged;
    }
    if tips.is_empty() {
        return RepoOutcome::Done(tips);
    }

    let mut cmd = git(dir);
    cmd.args([
        "log",
        "--no-color",
        "--ignore-missing",
        "--stdin",
        "--format=%at%x09%ai%x09%ae%x09%s",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped());
    out.configure_command(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        return RepoOutcome::Failed;
    };
    // git reads every revision before it prints anything, so writing all of
    // stdin first cannot deadlock.
    if let Some(mut stdin) = child.stdin.take() {
        let mut revs = String::new();
        for t in &tips {
            revs.push_str(t);
            revs.push('\n');
        }
        for t in old.into_iter().flatten() {
            revs.push('^');
            revs.push_str(t);
            revs.push('\n');
        }
        if stdin.write_all(revs.as_bytes()).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return RepoOutcome::Failed;
        }
    }
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return RepoOutcome::Failed;
    };
    let attrs = json!({ "repo": name }).to_string();
    let (mut read, mut matched) = (0u64, 0u64);
    let mut cancelled = false;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if out.cancelled() {
            cancelled = true;
            break;
        }
        let Some((id, t, email)) = parse_line(&line) else {
            continue;
        };
        read += 1;
        if emails.iter().any(|e| e.eq_ignore_ascii_case(email)) {
            matched += 1;
            out.item(id, t, true, attrs.clone());
        }
    }
    if cancelled {
        let _ = child.kill();
    }
    let ok = child.wait().is_ok_and(|s| s.success());
    out.count("commits_read", read);
    out.count("commits_matched", matched);
    if cancelled {
        RepoOutcome::Cancelled
    } else if ok {
        RepoOutcome::Done(tips)
    } else {
        RepoOutcome::Failed
    }
}

impl Collector for Git {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "Git",
            description: "Commit times and repository folder names for commits authored with your \
                          email addresses. Never code, diffs, commit messages, file names or line \
                          counts.",
            permissions: "Pick folders + author emails",
            cost: "One scan a day at background priority; after the first scan only new commits \
                   are read.",
            default_on: false,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Scanned
    }

    fn start(&mut self, _out: Sink) {}

    fn stop(&mut self) {}

    fn scanner(&self) -> Option<Box<dyn Collector>> {
        Some(Box::new(Git))
    }

    fn enable_defaults(&self, s: &Settings) -> Vec<(String, String)> {
        if s.git_author_emails.is_some() {
            return Vec::new();
        }
        vec![(
            KEY_GIT_EMAILS.to_string(),
            json!(global_author_email()).to_string(),
        )]
    }

    fn scan(&mut self, _since: Option<Timestamp>, out: Sink) {
        let settings = out.settings();
        let emails = settings
            .git_author_emails
            .clone()
            .unwrap_or_else(global_author_email);
        if emails.is_empty() {
            out.count("no_author_emails", 1);
        }
        let mut state = State::parse(out.scan_state());
        let salt = *state.salt.get_or_insert_with(scan::new_salt);
        let roots: Vec<PathBuf> = settings
            .git_folders
            .iter()
            .filter_map(|f| expand_home(f))
            .collect();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut cancelled = false;
        let walk = scan::walk(
            &roots,
            Rules::default(),
            |dir| {
                if out.cancelled() {
                    cancelled = true;
                    return Flow::Stop;
                }
                let Some((repo, name)) = repo_at(dir) else {
                    return Flow::Continue;
                };
                out.count("repos", 1);
                let key = format!("{:016x}", scan::path_hash(salt, &repo));
                seen.insert(key.clone());
                match scan_repo(&out, &repo, &name, state.repos.get(&key), &emails) {
                    RepoOutcome::Unchanged => out.count("repos_unchanged", 1),
                    RepoOutcome::Done(tips) => {
                        state.repos.insert(key, tips);
                        // Saved after the repo's items: a cancel or crash
                        // later never loses them, and never skips them.
                        out.save_scan_state(state.to_json());
                    }
                    RepoOutcome::Failed => out.count("repo_errors", 1),
                    RepoOutcome::Cancelled => {
                        cancelled = true;
                        return Flow::Stop;
                    }
                }
                // Bare repositories: nothing to walk inside.
                if repo.join(".git").exists() {
                    Flow::Continue
                } else {
                    Flow::SkipDir
                }
            },
            |_, _| Flow::Continue,
        );
        out.count("dir_errors", walk.errors);
        if !cancelled && !walk.stopped {
            // Forget repositories that are gone.
            state.repos.retain(|k, _| seen.contains(k));
        }
        out.save_scan_state(state.to_json());
    }

    fn metrics(&self) -> Vec<MetricDef> {
        vec![
            MetricDef {
                id: "commits",
                title: "Commits",
                unit: Unit::Count,
                key: None,
            },
            MetricDef {
                id: "commits_by_repo",
                title: "Commits by repository",
                unit: Unit::Count,
                key: Some("repository folder name"),
            },
            MetricDef {
                id: "commits_by_hour",
                title: "Commits by hour of day",
                unit: Unit::Count,
                key: Some("local hour 00..23"),
            },
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        let mut by_repo: BTreeMap<String, f64> = BTreeMap::new();
        let mut by_hour: BTreeMap<u32, f64> = BTreeMap::new();
        for it in day.items {
            *by_repo
                .entry(it.attr_str("repo").unwrap_or("(unknown)").to_string())
                .or_default() += 1.0;
            *by_hour.entry(it.local_hour()).or_default() += 1.0;
        }
        let mut out = Vec::new();
        if day.items.is_empty() {
            return out;
        }
        out.push(MetricValue::new("commits", "", day.items.len() as f64));
        for (k, v) in by_repo {
            out.push(MetricValue::new("commits_by_repo", k, v));
        }
        for (h, v) in by_hour {
            out.push(MetricValue::new("commits_by_hour", format!("{h:02}"), v));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_line() {
        let (id, t, e) =
            parse_line("1763300000\t2025-11-16 21:33:20 +0800\tMe@x.org\tFix\tthe thing").unwrap();
        assert_eq!(id.len(), 32);
        assert_eq!(t.offset_min, 480);
        assert_eq!(t.utc_ms, 1_763_300_000_000);
        assert_eq!(e, "Me@x.org");
        // Email case does not matter; subject, time and offset do.
        let same = parse_line("1763300000\t2025-11-16 21:33:20 +0800\tme@X.org\tFix\tthe thing");
        assert_eq!(same.unwrap().0, id);
        let reworded = parse_line("1763300000\t2025-11-16 21:33:20 +0800\tme@x.org\tFix it");
        assert_ne!(reworded.unwrap().0, id);
        let other_off = parse_line("1763300000\t2025-11-16 22:33:20 +0900\tme@x.org\tFix\tthe thing");
        assert_ne!(other_off.unwrap().0, id);
        assert!(parse_line("garbage").is_none());
    }

    #[test]
    fn state_roundtrip() {
        let mut s = State {
            salt: Some([1, u64::MAX]),
            ..Default::default()
        };
        s.repos.insert("ab".into(), vec!["c0".into()]);
        let back = State::parse(Some(&s.to_json()));
        assert_eq!(back.salt, s.salt);
        assert_eq!(back.repos, s.repos);
    }
}
