use std::path::Path;
use wrapped_core::windows_ipc as ipc;
pub use wrapped_core::windows_ipc::Event;
pub fn hold_db_lock(db: &Path) -> Result<std::fs::File, String> {
    ipc::lock(&db.with_added_extension("lock")).map_err(|e| format!("database writer lock: {e}"))
}
pub fn running_pid(db: &Path) -> Option<i32> {
    ipc::owner(&db.with_added_extension("lock"))
}
pub fn wake_for_ctl(db: &Path) -> bool {
    ipc::signal(&db.with_added_extension("lock"))
}
pub fn request_flush(db: &Path) -> Option<i32> {
    use wrapped_core::ctl::*;
    let pid = running_pid(db)?;
    matches!(
        call(
            db,
            &Request::new("flush", serde_json::Value::Null),
            &|| wake_for_ctl(db),
            REPLY_TIMEOUT
        ),
        Ok(Reply::Ok(_))
    )
    .then_some(pid)
}
