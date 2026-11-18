//! Embeds the build id (short git hash, "-dirty" when the tree has uncommitted
//! changes) so `wrapped-collector --version` can print it. `scripts/macos/bundle.sh`
//! exports `WRAPPED_BUILD_ID` with the same value it writes to CFBundleVersion;
//! a plain `cargo build` computes it here instead.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=WRAPPED_BUILD_ID");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
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
}
