//! Command channel from other processes (dashboard, CLI) to the running
//! collector, which is the single writer of the database.
//!
//! Platform-neutral: a request is a small JSON file in `<db>.ctl/`, the reply
//! another JSON file next to it. Nothing here knows how the collector is
//! woken up: the client passes a `wake` function (macOS: SIGUSR2 to the pid in
//! the lock file; Windows will need its own, see DEVIATIONS.md). The
//! collector also looks for requests at every minute tick, so a lost wake-up
//! delays a request by up to a minute instead of losing it.
//!
//! ```text
//! <db>.ctl/<id>.req.json   {"v":1,"id":"<id>","cmd":"set_settings","args":{...}}
//! <db>.ctl/<id>.rep.json   {"v":1,"id":"<id>","ok":true,"data":{...}}
//!                          {"v":1,"id":"<id>","ok":false,"code":"...","error":"..."}
//! ```
//!
//! Files are written under a temporary name and renamed into place, so a
//! reader never sees half a file. Requests older than [`MAX_AGE`] are deleted
//! unanswered (their client has given up).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

pub const VERSION: u32 = 1;
/// How long a client waits for the reply.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(8);
/// Requests older than this are dropped without an answer.
pub const MAX_AGE: Duration = Duration::from_secs(30);

pub const CMD_PING: &str = "ping";
pub const CMD_STATUS: &str = "status";
pub const CMD_SET_SETTINGS: &str = "set_settings";
pub const CMD_SET_COLLECTOR: &str = "set_collector";
pub const CMD_PAUSE: &str = "pause";
pub const CMD_SCAN: &str = "scan";
pub const CMD_DELETE_ALL: &str = "delete_all";
pub const CMD_FINISH_SETUP: &str = "finish_setup";
pub const CMD_AI_ADD_SOURCE: &str = "ai_add_source";
pub const CMD_AI_REMOVE_SOURCE: &str = "ai_remove_source";
pub const CMD_AI_SET_SOURCE: &str = "ai_set_source";

/// Text the user has to type before everything is deleted.
pub const DELETE_CONFIRMATION: &str = "delete";

/// The folder holding requests and replies for database `db`.
pub fn dir_for(db: &Path) -> PathBuf {
    db.with_extension("ctl")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub cmd: String,
    pub args: Value,
}

impl Request {
    pub fn new(cmd: &str, args: Value) -> Self {
        Self {
            cmd: cmd.to_string(),
            args,
        }
    }

    /// `{"v":1,"id":..,"cmd":..,"args":..}`
    pub fn to_json(&self, id: &str) -> Value {
        json!({ "v": VERSION, "id": id, "cmd": self.cmd, "args": self.args })
    }

    pub fn from_json(v: &Value) -> Result<Self, String> {
        let cmd = v
            .get("cmd")
            .and_then(Value::as_str)
            .filter(|c| !c.is_empty())
            .ok_or("request has no `cmd`")?;
        Ok(Self {
            cmd: cmd.to_string(),
            args: v.get("args").cloned().unwrap_or(Value::Null),
        })
    }
}

/// What the collector answers.
#[derive(Debug, Clone, PartialEq)]
pub enum Reply {
    Ok(Value),
    Err { code: String, error: String },
}

impl Reply {
    pub fn err(code: &str, error: impl Into<String>) -> Self {
        Reply::Err {
            code: code.to_string(),
            error: error.into(),
        }
    }

    pub fn to_json(&self, id: &str) -> Value {
        match self {
            Reply::Ok(data) => json!({ "v": VERSION, "id": id, "ok": true, "data": data }),
            Reply::Err { code, error } => {
                json!({ "v": VERSION, "id": id, "ok": false, "code": code, "error": error })
            }
        }
    }

    pub fn from_json(v: &Value) -> Result<Self, String> {
        match v.get("ok").and_then(Value::as_bool) {
            Some(true) => Ok(Reply::Ok(v.get("data").cloned().unwrap_or(Value::Null))),
            Some(false) => Ok(Reply::Err {
                code: v
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or("error")
                    .to_string(),
                error: v
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error")
                    .to_string(),
            }),
            None => Err("reply has no `ok`".into()),
        }
    }
}

/// Why a call did not produce a reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// No collector is running on this database.
    NotRunning,
    /// A collector was asked but did not answer in time.
    Timeout,
    Io(String),
}

impl CallError {
    /// Stable machine-readable code (the dashboard switches on it).
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotRunning => "not_running",
            Self::Timeout => "timeout",
            Self::Io(_) => "io",
        }
    }
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning => write!(f, "Wrapped isn't running"),
            Self::Timeout => write!(f, "Wrapped didn't answer in time"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for CallError {}

fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:x}-{:x}", std::process::id(), nanos)
}

/// Ids come from file names written by other processes: only plain ones are
/// ever used to build a path.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn write_atomic(path: &Path, v: &Value) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, v.to_string())?;
    std::fs::rename(&tmp, path)
}

/// Send `req` to the collector on `db` and wait for its reply.
///
/// `wake` should signal the running collector and return true, or return
/// false when none is running (then nothing is written and
/// [`CallError::NotRunning`] is returned at once).
pub fn call(
    db: &Path,
    req: &Request,
    wake: &dyn Fn() -> bool,
    timeout: Duration,
) -> Result<Reply, CallError> {
    let dir = dir_for(db);
    std::fs::create_dir_all(&dir)
        .map_err(|e| CallError::Io(format!("cannot create {}: {e}", dir.display())))?;
    let id = new_id();
    let req_path = dir.join(format!("{id}.req.json"));
    let rep_path = dir.join(format!("{id}.rep.json"));
    write_atomic(&req_path, &req.to_json(&id)).map_err(|e| CallError::Io(e.to_string()))?;
    if !wake() {
        let _ = std::fs::remove_file(&req_path);
        return Err(CallError::NotRunning);
    }
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(text) = std::fs::read_to_string(&rep_path) {
            let _ = std::fs::remove_file(&rep_path);
            let v: Value = serde_json::from_str(&text).map_err(|e| CallError::Io(e.to_string()))?;
            return Reply::from_json(&v).map_err(CallError::Io);
        }
        if Instant::now() >= deadline {
            // Withdraw it: a late answer to a client that left is waste.
            let _ = std::fs::remove_file(&req_path);
            return Err(CallError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Collector side: requests waiting, oldest first, each with its id. Stale
/// ones are deleted; unreadable ones are answered with an error.
pub fn take_pending(db: &Path) -> Vec<(String, Request)> {
    let dir = dir_for(db);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, String, PathBuf)> = Vec::new();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".req.json") else {
            continue;
        };
        if !valid_id(id) {
            continue;
        }
        let mtime = e
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(UNIX_EPOCH);
        found.push((mtime, id.to_string(), e.path()));
    }
    found.sort();
    let mut out = Vec::new();
    for (mtime, id, path) in found {
        if mtime.elapsed().is_ok_and(|age| age > MAX_AGE) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            .and_then(|v| Request::from_json(&v));
        match parsed {
            Ok(r) => out.push((id, r)),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                reply(db, &id, &Reply::err("bad_request", e));
            }
        }
    }
    out
}

/// Collector side: answer request `id` and remove the request.
pub fn reply(db: &Path, id: &str, reply: &Reply) {
    if !valid_id(id) {
        return;
    }
    let dir = dir_for(db);
    let _ = write_atomic(&dir.join(format!("{id}.rep.json")), &reply.to_json(id));
    let _ = std::fs::remove_file(dir.join(format!("{id}.req.json")));
}

/// Collector side, at startup: forget requests and replies of a previous run.
pub fn clear(db: &Path) {
    let _ = std::fs::remove_dir_all(dir_for(db));
}

/// The "open the dashboard on Settings" flag the tray leaves for the dashboard.
pub fn open_settings_flag(db: &Path) -> PathBuf {
    dir_for(db).join("open-settings")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_db(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wrapped-ctl-{tag}-{}", new_id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("wrapped.db")
    }

    #[test]
    fn not_running_leaves_nothing_behind() {
        let db = tmp_db("nr");
        let r = call(
            &db,
            &Request::new(CMD_PING, Value::Null),
            &|| false,
            Duration::from_millis(50),
        );
        assert_eq!(r, Err(CallError::NotRunning));
        assert!(take_pending(&db).is_empty());
    }

    #[test]
    fn round_trip() {
        let db = tmp_db("rt");
        let db2 = db.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..200 {
                let reqs = take_pending(&db2);
                if let Some((id, req)) = reqs.into_iter().next() {
                    reply(
                        &db2,
                        &id,
                        &Reply::Ok(json!({ "echo": req.cmd, "args": req.args })),
                    );
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let r = call(
            &db,
            &Request::new("hello", json!({ "a": 1 })),
            &|| true,
            Duration::from_secs(5),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(r, Reply::Ok(json!({ "echo": "hello", "args": { "a": 1 } })));
    }

    #[test]
    fn timeout_withdraws_the_request() {
        let db = tmp_db("to");
        let r = call(
            &db,
            &Request::new(CMD_PING, Value::Null),
            &|| true,
            Duration::from_millis(60),
        );
        assert_eq!(r, Err(CallError::Timeout));
        assert!(take_pending(&db).is_empty());
    }

    #[test]
    fn bad_and_hostile_files() {
        let db = tmp_db("bad");
        let dir = dir_for(&db);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("abc.req.json"), "not json").unwrap();
        // A name that is not a plain id is ignored, never used as a path.
        std::fs::write(dir.join("..%2f..%2fx.req.json"), "{}").unwrap();
        assert!(take_pending(&db).is_empty());
        let text = std::fs::read_to_string(dir.join("abc.rep.json")).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["code"], json!("bad_request"));
    }

    #[test]
    fn reply_json_shapes() {
        let v = Reply::err("x", "y").to_json("i");
        assert_eq!(Reply::from_json(&v).unwrap(), Reply::err("x", "y"));
        let v = Reply::Ok(json!([1])).to_json("i");
        assert_eq!(Reply::from_json(&v).unwrap(), Reply::Ok(json!([1])));
    }
}
