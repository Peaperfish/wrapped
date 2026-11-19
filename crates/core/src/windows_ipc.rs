//! The open file denies competing writers. Windows releases it on process exit.
//! A fresh kernel event name is published only after startup cleanup.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    os::windows::fs::OpenOptionsExt,
    path::Path,
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::Threading::*,
};

pub fn lock(path: &Path) -> io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(1)
        .open(path)
}

pub struct Event(HANDLE);
// Kernel event handles can be moved across threads. Event owns the close.
unsafe impl Send for Event {}
impl Event {
    pub fn handle(&self) -> HANDLE {
        self.0
    }
    pub fn wait(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, u32::MAX) == 0 }
    }
}

impl Drop for Event {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}
impl Event {
    pub fn publish(lock: &mut File) -> io::Result<Self> {
        let pid = std::process::id();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("Local\\Wrapped-{pid}-{nonce}{sequence:020}");
        let h = unsafe { CreateEventW(null_mut(), 0, 0, wide(&name).as_ptr()) };
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        let event = Self(h);
        write!(lock, "{pid}\n{name}")?;
        Ok(event)
    }
}
fn endpoint(db: &Path) -> Option<(i32, Event)> {
    let text = std::fs::read_to_string(db).ok()?;
    let (pid, name) = text.split_once('\n')?;
    let pid: i32 = pid.parse().ok()?;
    if pid <= 0 {
        return None;
    }
    let suffix = name.strip_prefix(&format!("Local\\Wrapped-{pid}-"))?;
    if suffix.is_empty() || !suffix.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, wide(name).as_ptr()) };
    if h.is_null() {
        None
    } else {
        Some((pid, Event(h)))
    }
}
pub fn owner(db: &Path) -> Option<i32> {
    endpoint(db).map(|e| e.0)
}
/// A tray click or a second launch may grant its foreground rights to the
/// existing dashboard. Its event listener then restores and focuses the window.
pub fn focus_dashboard(db: &Path) -> bool {
    endpoint(&db.with_added_extension("dashboard.lock")).is_some_and(|(pid, event)| unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid as u32);
        SetEvent(event.handle()) != 0
    })
}
pub fn signal(db: &Path) -> bool {
    endpoint(db).is_some_and(|(_, e)| unsafe { SetEvent(e.0) != 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writer_and_auto_reset_event_are_scoped_to_the_database_and_run() {
        let root = std::env::temp_dir().join(format!("Wrapped IPC 雪 {}", std::process::id()));
        let a = root.join("session.db");
        let b = root.join("session.sqlite");
        assert_ne!(crate::ctl::dir_for(&a), crate::ctl::dir_for(&b));
        let a = a.with_added_extension("lock");
        let b = b.with_added_extension("lock");
        {
            let mut first = lock(&a).unwrap();
            let ae = Event::publish(&mut first).unwrap();
            assert!(lock(&a).is_err());
            let mut second = lock(&b).unwrap();
            let be = Event::publish(&mut second).unwrap();
            assert!(signal(&a));
            assert_eq!(unsafe { WaitForSingleObject(ae.0, 100) }, 0);
            assert_eq!(unsafe { WaitForSingleObject(ae.0, 0) }, 258);
            assert_eq!(unsafe { WaitForSingleObject(be.0, 0) }, 258);
            assert_eq!(owner(&a), Some(std::process::id() as i32));
        }
        assert!(!signal(&a));
        {
            let mut restarted = lock(&a).unwrap();
            let event = Event::publish(&mut restarted).unwrap();
            assert!(signal(&a));
            assert_eq!(unsafe { WaitForSingleObject(event.0, 100) }, 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
