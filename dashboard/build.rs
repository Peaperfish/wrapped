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

fn main() {
    println!("cargo:rerun-if-env-changed=WRAPPED_BUILD_ID");
    println!("cargo:rerun-if-changed=../.git/HEAD");
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
