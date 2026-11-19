//! Requests from other processes (see [`crate::ctl`]), handled on the
//! collector's main thread. This is how the dashboard changes settings: the
//! collector stays the single writer of the database.
//!
//! Every handler validates before it writes, so a rejected request changes
//! nothing, and applies the change at once (no waiting for the minute tick).

use serde_json::{Map, Value, json};

use super::Engine;
use crate::collector::Kind;
use crate::ctl::{self, Reply, Request};
use crate::settings::{KEY_ONBOARDING_DONE, Pause, Settings, collector_enabled_key};

/// A failed request: a stable code and a message for the user.
struct Fail(&'static str, String);

type Res = std::result::Result<Value, Fail>;

fn invalid(msg: impl Into<String>) -> Fail {
    Fail("invalid", msg.into())
}

impl From<crate::store::Error> for Fail {
    fn from(e: crate::store::Error) -> Self {
        Fail("failed", e.to_string())
    }
}

impl Engine {
    /// Run one request and produce its reply. Never panics on bad input.
    pub fn handle_request(&mut self, req: &Request) -> Reply {
        let out = match req.cmd.as_str() {
            ctl::CMD_PING => Ok(json!({ "pong": true })),
            ctl::CMD_STATUS => Ok(self.request_status()),
            ctl::CMD_SET_SETTINGS => self.request_set_settings(&req.args),
            ctl::CMD_SET_COLLECTOR => self.request_set_collector(&req.args),
            ctl::CMD_PAUSE => self.request_pause(&req.args),
            ctl::CMD_SCAN => self.request_scan(&req.args),
            ctl::CMD_DELETE_ALL => self.request_delete_all(&req.args),
            ctl::CMD_FINISH_SETUP => self.request_finish_setup(&req.args),
            ctl::CMD_AI_ADD_SOURCE => self.request_ai_add_source(&req.args),
            ctl::CMD_AI_REMOVE_SOURCE => self.request_ai_remove_source(&req.args),
            ctl::CMD_AI_SET_SOURCE => self.request_ai_set_source(&req.args),
            other => Err(invalid(format!("unknown command `{other}`"))),
        };
        match out {
            Ok(v) => Reply::Ok(v),
            Err(Fail(code, msg)) => Reply::err(code, msg),
        }
    }

    fn request_status(&self) -> Value {
        let collectors: Map<String, Value> = self
            .slots
            .iter()
            .map(|s| (s.name.to_string(), json!(s.running)))
            .collect();
        json!({
            "setup": self.setup_mode,
            "paused": self.paused,
            "pause": serde_json::from_str::<Value>(&Settings::pause_json(self.settings.pause))
                .unwrap_or(Value::Null),
            "scanning": self.scans.iter().map(|s| s.name).collect::<Vec<_>>(),
            "collectors": collectors,
        })
    }

    /// `{"settings": {"<key>": <json value>, ...}}`. Collector on/off keys
    /// (`collector.<name>.enabled`) are accepted and keep recorded data when
    /// turned off; use `set_collector` to delete it.
    fn request_set_settings(&mut self, args: &Value) -> Res {
        let pairs = settings_pairs(args)?;
        self.apply_pairs(&pairs, false)?;
        self.after_settings_change()?;
        Ok(json!({ "changed": pairs.len() }))
    }

    fn request_set_collector(&mut self, args: &Value) -> Res {
        let name = args["name"]
            .as_str()
            .ok_or_else(|| invalid("`name` is missing"))?;
        let enabled = args["enabled"]
            .as_bool()
            .ok_or_else(|| invalid("`enabled` must be true or false"))?;
        let delete = args["delete_data"].as_bool().unwrap_or(false);
        self.slot_index(name)
            .map_err(|_| invalid(format!("unknown collector `{name}`")))?;
        let running = self.slots.iter().any(|s| s.name == name && s.running);
        if enabled {
            if !running {
                self.enable_collector(name)?;
            }
        } else {
            self.disable_collector(name, !delete)?;
        }
        self.after_settings_change()?;
        Ok(json!({}))
    }

    fn request_pause(&mut self, args: &Value) -> Res {
        let now = self.now();
        let pause = match args["mode"].as_str() {
            Some("off") => Pause::Off,
            Some("hour") => Pause::for_one_hour(now),
            Some("tomorrow") => Pause::until_tomorrow(now),
            Some("indefinite") => Pause::Indefinite,
            _ => return Err(invalid("`mode` must be off, hour, tomorrow or indefinite")),
        };
        self.set_pause(pause)?;
        Ok(json!({}))
    }

    /// `{"what": "git" | "photos" | "ai" | "all" | "<collector>"}`
    fn request_scan(&mut self, args: &Value) -> Res {
        let what = args["what"].as_str().unwrap_or("all");
        let scanned: Vec<&'static str> = self
            .slots
            .iter()
            .filter(|s| s.collector.kind() == Kind::Scanned)
            .map(|s| s.name)
            .collect();
        let names: Vec<&'static str> = match what {
            "all" => scanned,
            "ai" => vec!["ai_usage"],
            n => scanned.into_iter().filter(|s| *s == n).collect::<Vec<_>>(),
        };
        if names.is_empty() {
            return Err(invalid(format!("`{what}` is not a scanned collector")));
        }
        let mut started = Vec::new();
        let mut skipped = Map::new();
        for n in names {
            if self.setup_mode {
                skipped.insert(n.into(), json!("setup is not finished"));
                continue;
            }
            match self.start_scan(n) {
                Ok(true) => started.push(n),
                Ok(false) => {
                    skipped.insert(n.into(), json!("already running"));
                }
                Err(e) => {
                    skipped.insert(n.into(), json!(e.to_string()));
                }
            }
        }
        Ok(json!({ "started": started, "skipped": skipped }))
    }

    /// `{"confirm": "delete"}`: deletes every collector's data. Settings stay
    /// (collector toggles, folders, exclude list, AI sources, onboarding).
    fn request_delete_all(&mut self, args: &Value) -> Res {
        if args["confirm"].as_str() != Some(ctl::DELETE_CONFIRMATION) {
            return Err(invalid(format!(
                "type `{}` to confirm",
                ctl::DELETE_CONFIRMATION
            )));
        }
        let names: Vec<&'static str> = self.slots.iter().map(|s| s.name).collect();
        for n in names {
            self.delete_collector_data(n)?;
        }
        Ok(json!({}))
    }

    /// `{"settings": {...}, "collectors": {"<name>": bool, ...}}`: the
    /// first-launch checklist. Stores every choice explicitly, marks
    /// onboarding done and starts recording.
    fn request_finish_setup(&mut self, args: &Value) -> Res {
        if !self.setup_mode {
            return Err(invalid("setup is already finished"));
        }
        let mut pairs = settings_pairs(args)?;
        if let Some(c) = args["collectors"].as_object() {
            for (name, on) in c {
                let on = on
                    .as_bool()
                    .ok_or_else(|| invalid(format!("`collectors.{name}` must be true or false")))?;
                pairs.push((collector_enabled_key(name), on.to_string()));
            }
        }
        pairs.push((KEY_ONBOARDING_DONE.to_string(), "true".to_string()));
        self.apply_pairs(&pairs, false)?;
        self.setup_mode = false;
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)?;
        self.after_settings_change()?;
        self.log(|| "setup finished: recording".to_string());
        Ok(json!({}))
    }

    /// `{"tool": "<id>", "path": "<folder>"}`: a custom AI source (the folder
    /// holds that tool's logs). On macOS the folder is bookmarked too.
    fn request_ai_add_source(&mut self, args: &Value) -> Res {
        let tool = args["tool"]
            .as_str()
            .ok_or_else(|| invalid("`tool` is missing"))?;
        let path = args["path"]
            .as_str()
            .ok_or_else(|| invalid("`path` is missing"))?;
        let src = crate::ai::new_custom_source(&self.settings, tool, path, &self.scan_hooks)
            .map_err(invalid)?;
        let id = src.id.clone();
        let mut sources = self.settings.ai_sources.clone();
        sources.push(src);
        self.store_ai_sources(sources)?;
        // Read it soon (no-op while setup is unfinished, paused, or turned off).
        if !self.setup_mode && !self.paused {
            let _ = self.start_scan("ai_usage");
        }
        Ok(json!({ "id": id }))
    }

    /// `{"id": "<source id>"}`. A source is only ever removed by this request.
    fn request_ai_remove_source(&mut self, args: &Value) -> Res {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("`id` is missing"))?;
        let mut sources = self.settings.ai_sources.clone();
        let before = sources.len();
        sources.retain(|s| s.id != id);
        if sources.len() == before {
            return Err(invalid(format!("no source `{id}`")));
        }
        self.store_ai_sources(sources)?;
        Ok(json!({}))
    }

    /// `{"id": "<source id>", "enabled": bool}`
    fn request_ai_set_source(&mut self, args: &Value) -> Res {
        let id = args["id"]
            .as_str()
            .ok_or_else(|| invalid("`id` is missing"))?;
        let enabled = args["enabled"]
            .as_bool()
            .ok_or_else(|| invalid("`enabled` must be true or false"))?;
        let mut sources = self.settings.ai_sources.clone();
        let Some(s) = sources.iter_mut().find(|s| s.id == id) else {
            return Err(invalid(format!("no source `{id}`")));
        };
        s.enabled = enabled;
        self.store_ai_sources(sources)?;
        Ok(json!({}))
    }

    fn store_ai_sources(&mut self, sources: Vec<crate::settings::AiSource>) -> Result<(), Fail> {
        let mut s = self.settings.clone();
        s.ai_sources = sources;
        let json = s
            .get_json(crate::settings::KEY_AI_SOURCES)
            .map_err(|e| invalid(e.to_string()))?;
        self.store
            .set_setting(crate::settings::KEY_AI_SOURCES, &json)?;
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)?;
        Ok(())
    }

    /// Validate all pairs against a copy of the settings, then store them.
    /// Collector keys go through enable / disable so defaults are stored and
    /// scans stopped.
    fn apply_pairs(
        &mut self,
        pairs: &[(String, String)],
        delete_disabled: bool,
    ) -> Result<(), Fail> {
        let mut probe = self.settings.clone();
        for (k, raw) in pairs {
            if let Some(name) = k
                .strip_prefix("collector.")
                .and_then(|r| r.strip_suffix(".enabled"))
                && !self.slots.iter().any(|s| s.name == name)
            {
                return Err(invalid(format!("unknown collector `{name}`")));
            }
            probe.apply(k, raw).map_err(|e| invalid(e.to_string()))?;
        }
        // Plain settings first (a collector turned on reads them), then toggles.
        let mut toggles = Vec::new();
        for (k, raw) in pairs {
            match k
                .strip_prefix("collector.")
                .and_then(|r| r.strip_suffix(".enabled"))
            {
                Some(name) => toggles.push((name.to_string(), raw == "true")),
                None => {
                    self.store.set_setting(k, raw)?;
                }
            }
        }
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)?;
        for (name, on) in toggles {
            let running = self.slots.iter().any(|s| s.name == name && s.running);
            if on && !running {
                self.enable_collector(&name)?;
            } else if !on && running {
                self.disable_collector(&name, !delete_disabled)?;
            } else {
                // Already in that state: still store it, so the choice is explicit.
                self.store
                    .set_setting(&collector_enabled_key(&name), &on.to_string())?;
            }
        }
        let (s, _) = self.store.load_settings()?;
        self.apply_settings(s)?;
        Ok(())
    }

    /// After any change: start scans that are now due (a collector just turned
    /// on, a folder added) without waiting for the next minute tick.
    fn after_settings_change(&mut self) -> Result<(), Fail> {
        if self.auto_scans && !self.paused && !self.setup_mode {
            self.schedule_scans()?;
        }
        Ok(())
    }
}

/// `args.settings` as (key, raw JSON text) pairs.
fn settings_pairs(args: &Value) -> Result<Vec<(String, String)>, Fail> {
    let Some(m) = args["settings"].as_object() else {
        if args.get("settings").is_none() || args["settings"].is_null() {
            return Ok(Vec::new());
        }
        return Err(invalid("`settings` must be an object"));
    };
    Ok(m.iter().map(|(k, v)| (k.clone(), v.to_string())).collect())
}
