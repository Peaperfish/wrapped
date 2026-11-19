//! `wrapped-collector ctl` against a database nobody is running on: a clear
//! "not running" answer (exit code 3), and no request left behind.

use std::process::Command;

#[test]
fn ctl_without_a_collector_says_not_running() {
    let dir = std::env::temp_dir().join(format!("wrapped-ctl-cli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("wrapped.db");
    let out = Command::new(env!("CARGO_BIN_EXE_wrapped-collector"))
        .args(["ctl", "ping"])
        .env("WRAPPED_DB", &db)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["code"], "not_running");
    let left = std::fs::read_dir(dir.join("wrapped.ctl"))
        .map(|d| d.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "no request file left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ctl_rejects_arguments_that_are_not_json() {
    let out = Command::new(env!("CARGO_BIN_EXE_wrapped-collector"))
        .args(["ctl", "ping", "{nope"])
        .env("WRAPPED_DB", std::env::temp_dir().join("wrapped-ctl-x.db"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
