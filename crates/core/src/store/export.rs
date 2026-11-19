//! Export everything to JSON (one file) or CSV (one file per table).
//! Rows are streamed, so memory stays flat however large the database is.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde_json::{Value, json};

use super::{Result, Store};
use crate::time::Timestamp;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExportCounts {
    pub sessions: usize,
    pub daily_metrics: usize,
    pub samples: usize,
    pub settings: usize,
    pub items: usize,
    /// AI usage rows (hourly sums per tool / session / model / project).
    pub ai_usage: usize,
}

const AI_USAGE_COLS: &[&str] = &[
    "tool",
    "hour_utc_ms",
    "hour_local",
    "session",
    "model",
    "project",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "requests",
];

const SESSION_COLS: &[&str] = &[
    "collector",
    "key",
    "start_utc_ms",
    "start_offset_min",
    "start_local",
    "end_utc_ms",
    "end_offset_min",
    "end_local",
    "duration_secs",
    "open",
];
const METRIC_COLS: &[&str] = &["date", "collector", "metric", "key", "value"];
const SAMPLE_COLS: &[&str] = &[
    "collector",
    "metric",
    "time_utc_ms",
    "time_offset_min",
    "time_local",
    "value",
    "data",
];
const SETTING_COLS: &[&str] = &["key", "value"];
const ITEM_COLS: &[&str] = &[
    "collector",
    "id",
    "time_utc_ms",
    "time_offset_min",
    "time_local",
    "attrs",
];

impl Store {
    fn each_session(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT collector, key, start_ms, start_off, end_ms, end_off, open FROM sessions
             ORDER BY start_ms, collector",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let (s, e) = (
                Timestamp::new(r.get(2)?, r.get(3)?),
                Timestamp::new(r.get(4)?, r.get(5)?),
            );
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(r.get::<_, String>(1)?),
                json!(s.utc_ms),
                json!(s.offset_min),
                json!(s.to_rfc3339()),
                json!(e.utc_ms),
                json!(e.offset_min),
                json!(e.to_rfc3339()),
                json!((e.utc_ms - s.utc_ms) as f64 / 1000.0),
                json!(r.get::<_, i64>(6)? != 0),
            ])?;
            n += 1;
        }
        Ok(n)
    }

    fn each_metric(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT date, collector, metric, key, value FROM daily_metrics
             ORDER BY date, collector, metric, key",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(r.get::<_, String>(1)?),
                json!(r.get::<_, String>(2)?),
                json!(r.get::<_, String>(3)?),
                json!(r.get::<_, f64>(4)?),
            ])?;
            n += 1;
        }
        Ok(n)
    }

    fn each_sample(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT collector, metric, time_ms, time_off, value, data FROM samples
             ORDER BY time_ms, collector, metric",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let t = Timestamp::new(r.get(2)?, r.get(3)?);
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(r.get::<_, String>(1)?),
                json!(t.utc_ms),
                json!(t.offset_min),
                json!(t.to_rfc3339()),
                json!(r.get::<_, f64>(4)?),
                match r.get::<_, Option<String>>(5)? {
                    Some(d) => serde_json::from_str(&d).unwrap_or(Value::String(d)),
                    None => Value::Null,
                },
            ])?;
            n += 1;
        }
        Ok(n)
    }

    /// Items; `time_offset_min` is null (and `time_local` has no offset) when
    /// the source had no UTC offset.
    fn each_item(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT collector, id, time_ms, time_off, attrs FROM items
             ORDER BY time_ms, collector, id",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let off: Option<i32> = r.get(3)?;
            let t = Timestamp::new(r.get(2)?, off.unwrap_or(0));
            let local = match off {
                Some(_) => t.to_rfc3339(),
                None => t.to_rfc3339().trim_end_matches("+00:00").to_string(),
            };
            let attrs: String = r.get(4)?;
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(r.get::<_, String>(1)?),
                json!(t.utc_ms),
                json!(off),
                json!(local),
                serde_json::from_str(&attrs).unwrap_or(Value::String(attrs)),
            ])?;
            n += 1;
        }
        Ok(n)
    }

    /// AI usage: hourly sums per tool, session, model and project. Token counts
    /// and ids only; there is no text in the database to export.
    fn each_ai_usage(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT tool, hour_ms, off, session, model, project,
                    input, output, cache_read, cache_write, requests
             FROM ai_usage ORDER BY hour_ms, tool, session, model, project",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let t = Timestamp::new(r.get(1)?, r.get(2)?);
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(t.utc_ms),
                json!(t.to_rfc3339()),
                json!(r.get::<_, String>(3)?),
                json!(r.get::<_, String>(4)?),
                json!(r.get::<_, String>(5)?),
                json!(r.get::<_, i64>(6)?),
                json!(r.get::<_, i64>(7)?),
                json!(r.get::<_, i64>(8)?),
                json!(r.get::<_, i64>(9)?),
                json!(r.get::<_, i64>(10)?),
            ])?;
            n += 1;
        }
        Ok(n)
    }

    /// User settings only (no `internal.` rows), as stored JSON text.
    fn each_setting(&self, mut f: impl FnMut(Vec<Value>) -> Result<()>) -> Result<usize> {
        let mut st = self.conn.prepare(
            "SELECT key, value FROM settings WHERE key NOT LIKE 'internal.%' ORDER BY key",
        )?;
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            f(vec![
                json!(r.get::<_, String>(0)?),
                json!(r.get::<_, String>(1)?),
            ])?;
            n += 1;
        }
        Ok(n)
    }

    /// Write one JSON document: `{schema_version, exported_at, sessions: [..], ...}`,
    /// each table an array of objects.
    pub fn export_json(&self, w: &mut dyn Write, now: Timestamp) -> Result<ExportCounts> {
        let mut c = ExportCounts::default();
        write!(
            w,
            "{{\"schema_version\":{},\"exported_at\":{}",
            self.schema_version()?,
            json!(now.to_rfc3339())
        )?;
        type Each<'a> = fn(&'a Store, &mut dyn FnMut(Vec<Value>) -> Result<()>) -> Result<usize>;
        let tables: [(&str, &[&str], Each<'_>); 6] = [
            ("sessions", SESSION_COLS, |s, f| s.each_session(f)),
            ("daily_metrics", METRIC_COLS, |s, f| s.each_metric(f)),
            ("samples", SAMPLE_COLS, |s, f| s.each_sample(f)),
            ("settings", SETTING_COLS, |s, f| s.each_setting(f)),
            ("items", ITEM_COLS, |s, f| s.each_item(f)),
            ("ai_usage", AI_USAGE_COLS, |s, f| s.each_ai_usage(f)),
        ];
        for (i, (name, cols, each)) in tables.into_iter().enumerate() {
            write!(w, ",\"{name}\":[")?;
            let mut first = true;
            let n = each(self, &mut |vals| {
                let obj: serde_json::Map<String, Value> =
                    cols.iter().map(|c| c.to_string()).zip(vals).collect();
                if !first {
                    w.write_all(b",")?;
                }
                first = false;
                serde_json::to_writer(&mut *w, &obj).map_err(std::io::Error::from)?;
                Ok(())
            })?;
            w.write_all(b"]")?;
            match i {
                0 => c.sessions = n,
                1 => c.daily_metrics = n,
                2 => c.samples = n,
                3 => c.settings = n,
                4 => c.items = n,
                _ => c.ai_usage = n,
            }
        }
        w.write_all(b"}\n")?;
        w.flush()?;
        Ok(c)
    }

    /// Write `sessions.csv`, `daily_metrics.csv`, `samples.csv`,
    /// `settings.csv`, `items.csv` and `ai_usage.csv` into `dir` (created if needed).
    pub fn export_csv(&self, dir: &Path) -> Result<ExportCounts> {
        std::fs::create_dir_all(dir)?;
        let open = |name: &str, cols: &[&str]| -> Result<BufWriter<File>> {
            let mut w = BufWriter::new(File::create(dir.join(name))?);
            writeln!(w, "{}", cols.join(","))?;
            Ok(w)
        };
        let row = |w: &mut BufWriter<File>, vals: Vec<Value>| -> Result<()> {
            let cells: Vec<String> = vals.iter().map(csv_cell).collect();
            writeln!(w, "{}", cells.join(","))?;
            Ok(())
        };
        let mut w = open("sessions.csv", SESSION_COLS)?;
        let sessions = self.each_session(|v| row(&mut w, v))?;
        w.flush()?;
        let mut w = open("daily_metrics.csv", METRIC_COLS)?;
        let daily_metrics = self.each_metric(|v| row(&mut w, v))?;
        w.flush()?;
        let mut w = open("samples.csv", SAMPLE_COLS)?;
        let samples = self.each_sample(|v| row(&mut w, v))?;
        w.flush()?;
        let mut w = open("settings.csv", SETTING_COLS)?;
        let settings = self.each_setting(|v| row(&mut w, v))?;
        w.flush()?;
        let mut w = open("items.csv", ITEM_COLS)?;
        let items = self.each_item(|v| row(&mut w, v))?;
        w.flush()?;
        let mut w = open("ai_usage.csv", AI_USAGE_COLS)?;
        let ai_usage = self.each_ai_usage(|v| row(&mut w, v))?;
        w.flush()?;
        Ok(ExportCounts {
            sessions,
            daily_metrics,
            samples,
            settings,
            items,
            ai_usage,
        })
    }
}

fn csv_cell(v: &Value) -> String {
    let s = match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::OpenSession;
    use crate::time::{Clock, ManualClock};

    fn store_with_data() -> (Store, Timestamp) {
        let mut s = Store::open_in_memory().unwrap();
        s.set_setting("exclude.apps", r#"["a,b"]"#).unwrap();
        let now = ManualClock::at_local(2026, 11, 16, 9, 0, 0, 480).now();
        let open = [(
            "app_usage",
            OpenSession {
                key: "com.x".into(),
                start: now,
            },
        )];
        s.flush(
            &super::super::Batch {
                open: &open,
                ..Default::default()
            },
            now.plus_ms(60_000),
            &|_, _| Some(vec![]),
        )
        .unwrap();
        (s, now)
    }

    #[test]
    fn json_export_is_valid_and_complete() {
        let (s, now) = store_with_data();
        let mut buf = Vec::new();
        let c = s.export_json(&mut buf, now).unwrap();
        assert_eq!((c.sessions, c.settings), (1, 1));
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["sessions"][0]["key"], "com.x");
        assert_eq!(v["sessions"][0]["duration_secs"], 60.0);
        assert_eq!(v["sessions"][0]["start_local"], "2026-11-16T09:00:00+08:00");
        assert_eq!(v["settings"][0]["key"], "exclude.apps");
        assert!(v["daily_metrics"].as_array().unwrap().is_empty());
    }

    #[test]
    fn csv_export_quotes_cells() {
        let (s, _) = store_with_data();
        let dir = std::env::temp_dir().join(format!("wrapped-csv-{}", std::process::id()));
        let c = s.export_csv(&dir).unwrap();
        assert_eq!(c.sessions, 1);
        let settings = std::fs::read_to_string(dir.join("settings.csv")).unwrap();
        assert_eq!(settings, "key,value\nexclude.apps,\"[\"\"a,b\"\"]\"\n");
        let sessions = std::fs::read_to_string(dir.join("sessions.csv")).unwrap();
        assert!(
            sessions
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("app_usage,com.x,")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
