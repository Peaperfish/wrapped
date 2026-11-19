mod notify;
pub use notify::activate;
mod desktop;
mod ipc;
pub use ipc::{hold_db_lock, request_flush, running_pid, wake_for_ctl};
use std::{path::Path, sync::Arc};
use windows_sys::Win32::System::Threading::*;
pub use wrapped_core::windows_data_dir as data_dir;
use wrapped_core::{Engine, Store, SystemClock, collectors, ctl};

pub fn scan_hooks() -> wrapped_core::collector::ScanHooks {
    wrapped_core::collector::ScanHooks {
        thread_setup: Some(|| unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        }),
        command_setup: Some(|cmd| {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
        }),
        ..Default::default()
    }
}
pub struct Usage {
    pub cpu_secs: f64,
    pub disk_bytes_read: u64,
    pub disk_bytes_written: u64,
    pub peak_footprint: u64,
}
pub fn usage() -> Option<Usage> {
    None
}
pub fn children_cpu_secs() -> f64 {
    0.0
}
struct Probe;
impl wrapped_core::probe::SystemProbe for Probe {
    fn disk_memory(&self) -> Option<wrapped_core::probe::DiskMemoryReading> {
        disk_memory()
    }
}
pub fn disk_memory() -> Option<wrapped_core::probe::DiskMemoryReading> {
    use wrapped_core::{Clock, probe::*};
    let (read, write) = wrapped_core::now::windows::disk_totals()?;
    Some(DiskMemoryReading {
        disk_only: true,
        boot_time_ms: SystemClock.now().utc_ms
            - unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() } as i64,
        disk_bytes_read: read,
        disk_bytes_written: write,
        swap_in_bytes: 0,
        swap_out_bytes: 0,
        compressed_bytes: 0,
        pressure: PressureLevel::Normal,
    })
}

fn launch(db: &Path) -> Result<std::process::Child, String> {
    let bin = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("wrapped-dashboard.exe");
    wrapped_core::background_command(bin)
        .arg("--db")
        .arg(db)
        .spawn()
        .map_err(|e| e.to_string())
}
pub fn open_dashboard(db: &Path) -> Result<(), String> {
    if wrapped_core::windows_ipc::focus_dashboard(db) {
        return Ok(());
    }
    if running_pid(db).is_some() {
        match ctl::call(
            db,
            &ctl::Request::new("dashboard", serde_json::Value::Null),
            &|| wake_for_ctl(db),
            ctl::REPLY_TIMEOUT,
        ) {
            Ok(ctl::Reply::Ok(_)) => Ok(()),
            r => Err(format!("dashboard request: {r:?}")),
        }
    } else {
        launch(db).map(|_| ())
    }
}

pub fn login(cmd: &str) -> i32 {
    use winreg::{RegKey, enums::*};
    let result = (|| -> std::io::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
        let launcher = std::env::current_exe()?.with_file_name("Wrapped.exe");
        match cmd {
            "enable" => {
                if !launcher.is_file() {
                    return Err(std::io::Error::other("Wrapped.exe is missing"));
                }
                let mut command = std::ffi::OsString::from("\"");
                command.push(launcher);
                command.push("\"");
                if let Some(path) = std::env::var_os("WRAPPED_DB").filter(|s| !s.is_empty()) {
                    command.push(" --db \"");
                    command.push(std::path::absolute(path)?);
                    command.push("\"");
                }
                key.set_value("Wrapped", &command)?;
            }
            "disable" => {
                if let Err(e) = key.delete_value("Wrapped")
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(e);
                }
            }
            _ => println!(
                "{}",
                if key.get_value::<String, _>("Wrapped").is_ok() {
                    "enabled"
                } else {
                    "disabled"
                }
            ),
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("startup: {e}");
        1
    } else {
        0
    }
}

pub fn run_collector(db: &Path, debug: bool) -> i32 {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let mut lock = hold_db_lock(db)?;
        unsafe {
            if SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        let mut engine = Engine::new(
            Store::open(db)?,
            Arc::new(SystemClock),
            collectors::builtin(),
        )?;
        ctl::clear(db);
        let setup = engine.begin_onboarding()?;
        engine.set_probe(Arc::new(Probe));
        engine.set_own_app_id("wrapped-collector.exe");
        engine.set_scan_hooks(scan_hooks());
        if debug {
            engine.set_logger(|s| eprintln!("{s}"));
        }
        let event = ipc::Event::publish(&mut lock)?;
        let path = db.to_owned();
        engine.set_waker(Arc::new(move || {
            wake_for_ctl(&path);
        }));
        let engine = std::rc::Rc::new(std::cell::RefCell::new(engine));
        let result = desktop::run(db, &engine, &event, setup);
        engine.borrow_mut().shutdown()?;
        result?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("wrapped-collector: {e}");
        1
    } else {
        0
    }
}
