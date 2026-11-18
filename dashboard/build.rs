//! Tauri's build step (config, capabilities, Windows resources) plus the same
//! build id as the collector (short git hash, "-dirty" for uncommitted
//! changes; `WRAPPED_BUILD_ID` from `scripts/macos/bundle.sh` wins), so both
//! halves of Wrapped.app report one version.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Tells cargo to rerun when the commit or the index changes: HEAD, the
/// current branch ref, packed-refs and the index (which drives "-dirty").
/// Missing paths are skipped (a missing path would force a rerun every build,
/// and there is no .git at all in a source tarball).
fn watch_git(root: &str) {
    let dir = std::path::Path::new(root).join(".git");
    let head = dir.join("HEAD");
    let mut paths = vec![head.clone(), dir.join("packed-refs"), dir.join("index")];
    if let Ok(text) = std::fs::read_to_string(&head) {
        if let Some(r) = text.trim().strip_prefix("ref: ") {
            paths.push(dir.join(r));
        }
    }
    for p in paths.into_iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=WRAPPED_BUILD_ID");
    watch_git("..");
    let id = match std::env::var("WRAPPED_BUILD_ID") {
        Ok(v) if !v.is_empty() => v,
        _ => match git(&["rev-parse", "--short", "HEAD"]) {
            Some(hash) => {
                let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
                if dirty { format!("{hash}-dirty") } else { hash }
            }
            None => "unknown".to_string(),
        },
    };
    println!("cargo:rustc-env=WRAPPED_BUILD_ID={id}");
    tauri_build::build();
}
