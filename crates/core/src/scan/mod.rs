//! Helpers shared by the scanned collectors: a streaming folder walk with the
//! skip rules, SipHash-2-4 for stable keyed hashes, and a random salt.
//! No platform code: only `std::fs`.

pub mod exif;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// What a walk callback wants next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Go on (for a directory: descend into it).
    Continue,
    /// Directory only: do not descend.
    SkipDir,
    /// Stop the whole walk (cancelled).
    Stop,
}

/// Skip rules for both scanned collectors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Rules {
    /// Also skip `*.photoslibrary` packages (photos).
    pub skip_photo_libraries: bool,
}

impl Rules {
    /// Hidden entries (including `.git`, `.venv`), `node_modules`, `target`,
    /// `build*` and, for photos, `*.photoslibrary`.
    pub fn skip_dir(&self, name: &str) -> bool {
        name.starts_with('.')
            || name == "node_modules"
            || name == "target"
            || name.starts_with("build")
            || (self.skip_photo_libraries && name.to_ascii_lowercase().ends_with(".photoslibrary"))
    }
}

/// A regular file met by the walk.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub size: u64,
    pub mtime_ms: i64,
}

/// What the walk counted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WalkStats {
    pub dirs: u64,
    /// Directories that could not be read (permission, vanished, ...).
    pub errors: u64,
    pub stopped: bool,
}

/// Depth-first walk of `roots`, streaming: memory is the stack of pending
/// directories plus one directory listing. Symlinks are never followed (nor
/// reported); hidden files are not reported; entries are visited in name
/// order so walks are repeatable.
///
/// `on_dir(path)` is called for every directory (roots included) before its
/// children; `on_file(path, info)` for every regular, non-hidden file.
pub fn walk(
    roots: &[PathBuf],
    rules: Rules,
    mut on_dir: impl FnMut(&Path) -> Flow,
    mut on_file: impl FnMut(&Path, &FileInfo) -> Flow,
) -> WalkStats {
    let mut st = WalkStats::default();
    let mut stack: Vec<PathBuf> = roots.iter().rev().cloned().collect();
    while let Some(dir) = stack.pop() {
        match on_dir(&dir) {
            Flow::Continue => {}
            Flow::SkipDir => continue,
            Flow::Stop => {
                st.stopped = true;
                return st;
            }
        }
        st.dirs += 1;
        let Ok(rd) = fs::read_dir(&dir) else {
            st.errors += 1;
            continue;
        };
        let mut subdirs = Vec::new();
        let mut files = Vec::new();
        for ent in rd.flatten() {
            let name = ent.file_name();
            let Some(name) = name.to_str() else { continue };
            // `file_type` does not follow symlinks.
            let Ok(ft) = ent.file_type() else { continue };
            if ft.is_dir() {
                if !rules.skip_dir(name) {
                    subdirs.push(ent.path());
                }
            } else if ft.is_file() && !name.starts_with('.') {
                files.push(ent.path());
            }
        }
        files.sort();
        for f in files {
            let Ok(md) = fs::symlink_metadata(&f) else {
                continue;
            };
            let mtime_ms = md
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let info = FileInfo {
                size: md.len(),
                mtime_ms,
            };
            if on_file(&f, &info) == Flow::Stop {
                st.stopped = true;
                return st;
            }
        }
        subdirs.sort();
        stack.extend(subdirs.into_iter().rev());
    }
    st
}

/// SipHash-2-4 (Aumasson & Bernstein), 64-bit output. Implemented here so
/// stored hashes never change with a library or compiler version.
pub fn siphash24(k0: u64, k1: u64, data: &[u8]) -> u64 {
    let mut v0 = k0 ^ 0x736f_6d65_7073_6575;
    let mut v1 = k1 ^ 0x646f_7261_6e64_6f6d;
    let mut v2 = k0 ^ 0x6c79_6765_6e65_7261;
    let mut v3 = k1 ^ 0x7465_6462_7974_6573;
    let round = |v0: &mut u64, v1: &mut u64, v2: &mut u64, v3: &mut u64| {
        *v0 = v0.wrapping_add(*v1);
        *v1 = v1.rotate_left(13) ^ *v0;
        *v0 = v0.rotate_left(32);
        *v2 = v2.wrapping_add(*v3);
        *v3 = v3.rotate_left(16) ^ *v2;
        *v0 = v0.wrapping_add(*v3);
        *v3 = v3.rotate_left(21) ^ *v0;
        *v2 = v2.wrapping_add(*v1);
        *v1 = v1.rotate_left(17) ^ *v2;
        *v2 = v2.rotate_left(32);
    };
    let (chunks, tail) = data.as_chunks::<8>();
    for c in chunks {
        let m = u64::from_le_bytes(*c);
        v3 ^= m;
        round(&mut v0, &mut v1, &mut v2, &mut v3);
        round(&mut v0, &mut v1, &mut v2, &mut v3);
        v0 ^= m;
    }
    let mut last = (data.len() as u64 & 0xff) << 56;
    for (i, b) in tail.iter().enumerate() {
        last |= u64::from(*b) << (8 * i);
    }
    v3 ^= last;
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    round(&mut v0, &mut v1, &mut v2, &mut v3);
    v0 ^= last;
    v2 ^= 0xff;
    for _ in 0..4 {
        round(&mut v0, &mut v1, &mut v2, &mut v3);
    }
    v0 ^ v1 ^ v2 ^ v3
}

/// 128-bit hex digest from two SipHash-2-4 runs with fixed keys (dedupe ids).
pub fn hash128_hex(data: &[u8]) -> String {
    const K: [u64; 4] = [
        0x7772_6170_7065_6431,
        0x7368_6f74_2d69_6431,
        0x7772_6170_7065_6432,
        0x7368_6f74_2d69_6432,
    ];
    format!(
        "{:016x}{:016x}",
        siphash24(K[0], K[1], data),
        siphash24(K[2], K[3], data)
    )
}

/// A per-database secret key for hashing paths, so stored hashes cannot be
/// matched against guessed paths. Random from the OS-seeded std hasher keys.
pub fn new_salt() -> [u64; 2] {
    use std::hash::{BuildHasher, Hasher};
    let pick = |n: u64| {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(n);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        );
        h.finish()
    };
    [pick(1), pick(2)]
}

/// Salted 64-bit hash of a path (the only form in which paths are stored).
pub fn path_hash(salt: [u64; 2], path: &Path) -> u64 {
    siphash24(salt[0], salt[1], path.as_os_str().as_encoded_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siphash_reference_vector() {
        // From the SipHash paper: key 00..0f, message 00..0e.
        let k: Vec<u8> = (0u8..16).collect();
        let k0 = u64::from_le_bytes(k[0..8].try_into().unwrap());
        let k1 = u64::from_le_bytes(k[8..16].try_into().unwrap());
        let msg: Vec<u8> = (0u8..15).collect();
        assert_eq!(siphash24(k0, k1, &msg), 0xa129_ca61_49be_45e5);
        // Empty message (vectors table, first entry: 310e0edd47db6f72).
        assert_eq!(siphash24(k0, k1, &[]), 0x726f_db47_dd0e_0e31);
    }

    #[test]
    fn salts_differ() {
        assert_ne!(new_salt(), new_salt());
    }

    #[test]
    fn skip_rules() {
        let r = Rules::default();
        for n in [
            ".git",
            ".venv",
            "node_modules",
            "target",
            "build",
            "build-x",
        ] {
            assert!(r.skip_dir(n), "{n}");
        }
        assert!(!r.skip_dir("src"));
        assert!(!r.skip_dir("Photos Library.photoslibrary"));
        let p = Rules {
            skip_photo_libraries: true,
        };
        assert!(p.skip_dir("Photos Library.photoslibrary"));
    }

    #[cfg(unix)]
    #[test]
    fn walk_skips_and_does_not_follow_symlinks() {
        let root = std::env::temp_dir().join(format!("wrapped-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for d in ["a/b", "node_modules/x", ".hidden", "build2", "c"] {
            fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in [
            "a/1.jpg",
            "a/b/2.jpg",
            "node_modules/x/3.jpg",
            ".hidden/4.jpg",
            "build2/5.jpg",
            "a/.6.jpg",
        ] {
            fs::write(root.join(f), b"x").unwrap();
        }
        std::os::unix::fs::symlink(root.join("a"), root.join("c/link")).unwrap();
        std::os::unix::fs::symlink(root.join("a/1.jpg"), root.join("c/7.jpg")).unwrap();
        let mut seen = Vec::new();
        let st = walk(
            std::slice::from_ref(&root),
            Rules::default(),
            |_| Flow::Continue,
            |p, _| {
                seen.push(
                    p.strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
                Flow::Continue
            },
        );
        assert_eq!(seen, vec!["a/1.jpg", "a/b/2.jpg"]);
        assert_eq!(st.errors, 0);
        let _ = fs::remove_dir_all(&root);
    }
}
