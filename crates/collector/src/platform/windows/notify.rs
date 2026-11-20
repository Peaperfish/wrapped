//! Toast activation survives collector restarts. A link names a registered
//! database token and a canonical recap id; it cannot supply an arbitrary path.
use ::windows::{
    Data::Xml::Dom::XmlDocument,
    UI::Notifications::{ToastNotification, ToastNotificationManager},
    core::HSTRING,
};
use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
};
use winreg::{RegKey, enums::*};
use wrapped_core::{
    Clock, Engine, SystemClock, collectors,
    recap::{self, Period, PeriodKind, notify as rn},
    stats::Stats,
};
const APP: &str = "Peaperfish.Wrapped";
const DATABASES: &str = r"Software\Peaperfish\Wrapped\Databases";
fn register(db: &Path) -> std::io::Result<String> {
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let path = db.canonicalize()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    let token = format!("{:016x}", hash.finish());
    root.create_subkey(DATABASES)?
        .0
        .set_value(&token, &path.as_os_str())?;
    root.create_subkey(r"Software\Classes\AppUserModelId\Peaperfish.Wrapped")?
        .0
        .set_value("DisplayName", &"Wrapped")?;
    let scheme = root.create_subkey(r"Software\Classes\wrapped-recap")?.0;
    scheme.set_value("", &"URL:Wrapped recap")?;
    scheme.set_value("URL Protocol", &"")?;
    let bin = std::env::current_exe()?.with_file_name("Wrapped.exe");
    let mut command = std::ffi::OsString::from("\"");
    command.push(bin);
    command.push("\" activate \"%1\"");
    scheme
        .create_subkey(r"shell\open\command")?
        .0
        .set_value("", &command)?;
    Ok(token)
}
fn route(uri: &str) -> Option<(&str, &str)> {
    let (token, id) = uri.strip_prefix("wrapped-recap:")?.split_once('/')?;
    (token.len() == 16
        && token.bytes().all(|b| b.is_ascii_hexdigit())
        && Period::parse_id(id)?.id() == id)
        .then_some((token, id))
}
pub fn activate(uri: &str) -> Result<(), String> {
    let (token, id) = route(uri).ok_or("invalid Wrapped recap link")?;
    let dir = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(DATABASES)
        .map_err(|e| e.to_string())?;
    let db: std::ffi::OsString = dir.get_value(token).map_err(|e| e.to_string())?;
    activate_db(&PathBuf::from(db), id)
}
fn activate_db(db: &Path, id: &str) -> Result<(), String> {
    std::fs::create_dir_all(wrapped_core::ctl::dir_for(db))
        .and_then(|_| std::fs::write(wrapped_core::ctl::open_recap_flag(db), id))
        .map_err(|e| e.to_string())?;
    super::open_dashboard(db)
}
fn post(db: &Path, notice: &rn::Notice) -> Result<(), Box<dyn std::error::Error>> {
    let token = register(db)?;
    let xml = XmlDocument::new()?;
    let body = notice
        .body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    xml.LoadXml(&HSTRING::from(format!(r#"<toast activationType="protocol" launch="wrapped-recap:{token}/{}"><visual><binding template="ToastGeneric"><text>Wrapped</text><text>{body}</text></binding></visual></toast>"#,notice.period.id())))?;
    let toast = ToastNotification::CreateToastNotification(&xml)?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP))?.Show(&toast)?;
    Ok(())
}
pub fn permission(db: &Path, e: &Engine) -> bool {
    let permission = register(db).ok().and_then(|_| {
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP))
            .ok()?
            .Setting()
            .ok()
    });
    let state = match permission {
        Some(::windows::UI::Notifications::NotificationSetting::Enabled) => "granted",
        Some(_) => "denied",
        None => "unavailable",
    };
    if e.store()
        .internal_value(rn::PERMISSION_KEY)
        .ok()
        .flatten()
        .as_deref()
        != Some(state)
    {
        let _ = e.store().set_internal_value(rn::PERMISSION_KEY, state);
    }
    state == "granted"
}
pub fn check(db: &Path, e: &Engine) {
    if !permission(db, e) {
        return;
    }
    match rn::check(
        e.store(),
        e.settings(),
        &collectors::builtin(),
        SystemClock.now(),
        str::to_owned,
    ) {
        Ok(ns) => {
            for n in ns {
                if let Err(e) = post(db, &n) {
                    eprintln!("notification: {e}")
                }
            }
        }
        Err(e) => eprintln!("recap check: {e}"),
    }
}
pub fn test(
    db: &Path,
    e: &Engine,
    args: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let kind =
        PeriodKind::parse(args["kind"].as_str().unwrap_or("week")).ok_or("invalid recap kind")?;
    let all = collectors::builtin();
    let stats = Stats::new(e.store(), &all, e.settings());
    let today = SystemClock.now().local_date();
    let entry = recap::list(&stats, today)?
        .into_iter()
        .find(|e| e.period.kind == kind)
        .ok_or("no recap available")?;
    let recap = recap::build(&stats, entry.period, today)?.ok_or("recap unavailable")?;
    post(db, &rn::notice(&recap, &str::to_owned))?;
    Ok(serde_json::json!({"posted":entry.period.id()}))
}
#[cfg(test)]
mod tests {
    #[test]
    fn click_writes_the_selected_recap_and_wakes_its_dashboard() {
        use wrapped_core::{
            ctl,
            windows_ipc::{Event, lock},
        };
        let root = std::env::temp_dir().join(format!("Wrapped activation {}", std::process::id()));
        let db = root.join("session.雪");
        {
            let mut file = lock(&db.with_added_extension("dashboard.lock")).unwrap();
            let signal = Event::publish(&mut file).unwrap();
            let id = "week:2026-11-09";
            super::activate_db(&db, id).unwrap();
            assert_eq!(
                unsafe {
                    windows_sys::Win32::System::Threading::WaitForSingleObject(signal.handle(), 100)
                },
                0
            );
            assert_eq!(
                std::fs::read_to_string(ctl::open_recap_flag(&db)).unwrap(),
                id
            );
            assert!(!ctl::open_recap_flag(&root.join("session.db")).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activation_is_scoped_to_registered_database_and_recap() {
        let token = "0123456789abcdef";
        let id = "week:2026-11-09";
        assert_eq!(
            super::route(&format!("wrapped-recap:{token}/{id}")),
            Some((token, id))
        );
        assert!(super::route("wrapped-recap:../wrapped.db/week:2026-11-09").is_none());
        assert!(super::route(&format!("wrapped-recap:{token}/{id}/x")).is_none());
    }
}
