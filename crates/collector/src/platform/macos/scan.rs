//! Scan support: background priority for scan threads and the processes they
//! start, and this process's resource usage for the `scan` report.

use std::os::unix::process::CommandExt;
use std::process::Command;

use wrapped_core::collector::ScanHooks;

/// `QOS_CLASS_BACKGROUND` from `<sys/qos.h>` (not in the libc crate).
const QOS_CLASS_BACKGROUND: u32 = 0x09;

unsafe extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: libc::c_int)
    -> libc::c_int;
}

/// Background QoS for the calling thread: lowest CPU priority and throttled
/// disk I/O (the scheduler runs it only when nothing else wants the CPU).
fn background_thread() {
    let rc = unsafe { pthread_set_qos_class_self_np(QOS_CLASS_BACKGROUND, 0) };
    if rc != 0 {
        super::log(&format!(
            "could not set background QoS for scan thread ({rc})"
        ));
    }
}

/// Child processes (git) run as background processes too.
fn background_command(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            // Async-signal-safe: a single syscall between fork and exec.
            libc::setpriority(libc::PRIO_DARWIN_PROCESS, 0, libc::PRIO_DARWIN_BG);
            Ok(())
        });
    }
}

pub fn scan_hooks() -> ScanHooks {
    ScanHooks {
        thread_setup: Some(background_thread),
        command_setup: Some(background_command),
        bookmark_create: Some(super::bookmarks::create),
        bookmark_resolve: Some(super::bookmarks::resolve),
        drive_health: Some(super::drive_health::read),
    }
}

/// Resource usage of this process so far.
pub struct Usage {
    pub cpu_secs: f64,
    pub disk_bytes_read: u64,
    pub disk_bytes_written: u64,
    pub peak_footprint: u64,
}

pub fn usage() -> Option<Usage> {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V4,
            (&mut info as *mut libc::rusage_info_v4).cast(),
        )
    };
    if rc != 0 {
        return None;
    }
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    Some(Usage {
        cpu_secs: tv(ru.ru_utime) + tv(ru.ru_stime),
        disk_bytes_read: info.ri_diskio_bytesread,
        disk_bytes_written: info.ri_diskio_byteswritten,
        peak_footprint: info.ri_lifetime_max_phys_footprint,
    })
}

/// Children's CPU time (git processes), seconds.
pub fn children_cpu_secs() -> f64 {
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut ru) };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    tv(ru.ru_utime) + tv(ru.ru_stime)
}

/// Hold the database lock for a CLI scan, so a collector cannot start on the
/// same database meanwhile. SIGUSR1 (sent by `stats` etc. to "the collector")
/// is ignored instead of killing this process.
/// Held while a CLI scan runs; dropping it releases the database lock.
pub struct DbLock {
    _lock: super::lock::Lock,
}

pub fn hold_db_lock(db: &std::path::Path) -> Result<DbLock, String> {
    let lock = match super::lock::acquire(&super::lock_path(db)) {
        Ok(l) => l,
        Err(super::lock::LockError::Held(pid)) => {
            return Err(format!(
                "a collector is running on this database (pid {pid})"
            ));
        }
        Err(super::lock::LockError::Io(e)) => return Err(format!("cannot create lock file: {e}")),
    };
    unsafe { libc::signal(libc::SIGUSR1, libc::SIG_IGN) };
    Ok(DbLock { _lock: lock })
}
