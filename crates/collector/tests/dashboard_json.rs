//! The dashboard's `summary(from, to)` returns exactly what
//! `wrapped-collector stats range <from> <to> --json` prints, for the same
//! database and range (both are `Summary::to_json()`; this guards against
//! either side drifting, e.g. a different top-N or settings handling).

#[path = "../../../dashboard/tests/fixture/mod.rs"]
mod fixture;

use std::process::Command;

use serde_json::Value;

fn cli_json(db: &std::path::Path, from: &str, to: &str) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_wrapped-collector"))
        .args(["stats", "range", from, to, "--json"])
        .env("WRAPPED_DB", db)
        .env_remove("WRAPPED_DEBUG")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stats range failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

/// Equal JSON; numbers compared as f64 with a relative tolerance of 1e-12
/// (the CLI's text goes through a decimal round trip).
fn same(a: &Value, b: &Value, path: &str) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            assert!(
                (x - y).abs() <= 1e-12 * x.abs().max(y.abs()),
                "{path}: {x} != {y}"
            );
        }
        (Value::Array(x), Value::Array(y)) => {
            assert_eq!(x.len(), y.len(), "{path} length");
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, &format!("{path}[{i}]"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            assert_eq!(
                x.keys().collect::<Vec<_>>(),
                y.keys().collect::<Vec<_>>(),
                "{path} keys"
            );
            for (k, p) in x {
                same(p, &y[k], &format!("{path}.{k}"));
            }
        }
        _ => assert_eq!(a, b, "{path}"),
    }
}

#[test]
fn dashboard_summary_equals_cli_stats_range_json() {
    let dir = fixture::temp_dir("cli-json");
    let db = fixture::build(&dir);
    for (from, to) in [
        ("2025-12-30", "2026-03-10"), // everything, across a new year
        ("2026-03-09", "2026-03-09"), // one day
        ("2026-01-01", "2026-12-31"), // a full year
        ("2024-01-01", "2024-12-31"), // a year without data
    ] {
        let dash = wrapped_dashboard::data::summary(&db, from, to).unwrap();
        same(&dash, &cli_json(&db, from, to), &format!("{from}..{to}"));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
