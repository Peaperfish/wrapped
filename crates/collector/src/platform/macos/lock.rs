//! Single-instance lock: `flock` on `wrapped.lock` next to the database, with
//! the pid written inside. The kernel drops the lock when the process exits,
//! however it exits, so a stale file never blocks a restart.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

pub enum LockError {
    Held(i32),
    Io(std::io::Error),
}

/// Held for the life of the process.
pub struct Lock {
    _file: File,
}

fn read_pid(f: &mut File) -> i32 {
    let mut s = String::new();
    let _ = f.rewind();
    let _ = f.read_to_string(&mut s);
    s.trim().parse().unwrap_or(0)
}

pub fn acquire(path: &Path) -> Result<Lock, LockError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(LockError::Io)?;
    }
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(LockError::Io)?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(LockError::Held(read_pid(&mut f)));
    }
    f.set_len(0).map_err(LockError::Io)?;
    f.rewind().map_err(LockError::Io)?;
    write!(f, "{}", std::process::id()).map_err(LockError::Io)?;
    Ok(Lock { _file: f })
}

/// Pid of the collector holding the lock, if one is running.
pub fn running_pid(path: &Path) -> Option<i32> {
    let mut f = File::open(path).ok()?;
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } == 0 {
        unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_UN) };
        return None; // nobody holds it
    }
    Some(read_pid(&mut f)).filter(|p| *p > 0)
}
