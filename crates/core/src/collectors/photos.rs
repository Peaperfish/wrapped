//! Photos: EXIF only (capture time, camera, lens, settings) from image files
//! in the chosen folders.
//!
//! Per shot it records: capture time, camera make/model, lens model, focal
//! length, aperture, shutter speed and ISO. Never pixel data, file paths or
//! file names. A shot's id hashes (make, model, body serial, DateTimeOriginal,
//! subsec, image number), so a RAW+JPEG pair, a copy, or a moved / renamed
//! file counts once. Files without a capture date are skipped (counted as
//! `no_date` in the scan report).
//!
//! Day: the capture's local date, using OffsetTimeOriginal when present,
//! otherwise the camera time as written (stored with an unknown offset).
//!
//! Incremental: a per-file cache of (salted path hash, size, mtime) in
//! `scan_files`; unchanged files are not opened. Reads are capped per file
//! (see [`crate::scan::exif`]) and the walk is streamed.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;

use chrono::NaiveDateTime;
use serde_json::{Map, Value, json};

use crate::collector::{
    Agg,
    Collector, CollectorInfo, DayInput, DayItem, FileStamp, Kind, MetricDef, MetricValue, Sink,
    Unit,
};
use crate::scan::exif::{Capped, HARD_CAP, MetaError, PhotoMeta, read_meta};
use crate::scan::{self, Flow, Rules};
use crate::settings::expand_home;
use crate::time::{MS_PER_MIN, Timestamp};

pub const NAME: &str = "photos";
const UNKNOWN: &str = "(unknown)";

#[derive(Default)]
pub struct Photos;

fn parse_salt(state: Option<&str>) -> Option<[u64; 2]> {
    let v: Value = serde_json::from_str(state?).ok()?;
    let a: Vec<u64> = v["salt"]
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str()?.parse().ok())
        .collect();
    (a.len() == 2).then(|| [a[0], a[1]])
}

/// Capture moment: (timestamp, offset known).
fn capture_time(m: &PhotoMeta, taken: NaiveDateTime) -> (Timestamp, bool) {
    let local_ms = taken.and_utc().timestamp_millis() + m.subsec_ms();
    match m.offset_min {
        Some(off) => (
            Timestamp::new(local_ms - i64::from(off) * MS_PER_MIN, off),
            true,
        ),
        None => (Timestamp::new(local_ms, 0), false),
    }
}

fn attrs(m: &PhotoMeta) -> String {
    let mut o = Map::new();
    let mut put = |k: &str, v: Value| {
        if !v.is_null() {
            o.insert(k.to_string(), v);
        }
    };
    put("make", json!(m.make));
    put("model", json!(m.model));
    put("lens", json!(m.lens));
    put("focal_mm", json!(m.focal_mm.map(|f| (f * 100.0).round() / 100.0)));
    put("f_number", json!(m.f_number.map(|f| (f * 100.0).round() / 100.0)));
    put("exposure_s", json!(m.exposure_s));
    put("iso", json!(m.iso));
    Value::Object(o).to_string()
}

/// "SONY ILCE-7M4"; "Canon EOS R5" (model already names the make).
pub fn camera_label(make: Option<&str>, model: Option<&str>) -> String {
    match (make, model) {
        (Some(mk), Some(md)) if md.to_lowercase().starts_with(&mk.to_lowercase()) => md.into(),
        (Some(mk), Some(md)) => {
            // "NIKON CORPORATION" + "NIKON Z 6" → "NIKON Z 6".
            let first = mk.split_whitespace().next().unwrap_or(mk);
            if md.to_lowercase().starts_with(&first.to_lowercase()) {
                md.into()
            } else {
                format!("{mk} {md}")
            }
        }
        (None, Some(md)) => md.into(),
        (Some(mk), None) => mk.into(),
        (None, None) => UNKNOWN.into(),
    }
}

fn camera_of(it: &DayItem) -> String {
    camera_label(it.attr_str("make"), it.attr_str("model"))
}

impl Collector for Photos {
    fn info(&self) -> CollectorInfo {
        CollectorInfo {
            name: NAME,
            title: "Photos",
            description: "From the EXIF of photos in the folders you pick: capture time, camera, \
                          lens, focal length, aperture, shutter speed and ISO. Never the image \
                          itself, file names or paths.",
            permissions: "Pick folders (macOS asks once for Pictures)",
            cost: "One scan a day at background priority; only new or changed files are opened, \
                   and only their metadata is read.",
            default_on: false,
        }
    }

    fn kind(&self) -> Kind {
        Kind::Scanned
    }

    fn start(&mut self, _out: Sink) {}

    fn stop(&mut self) {}

    fn scanner(&self) -> Option<Box<dyn Collector>> {
        Some(Box::new(Photos))
    }

    fn scan(&mut self, _since: Option<Timestamp>, out: Sink) {
        let settings = out.settings();
        let salt = match parse_salt(out.scan_state()) {
            Some(s) => s,
            None => {
                let s = scan::new_salt();
                out.save_scan_state(json!({ "salt": [s[0].to_string(), s[1].to_string()] }).to_string());
                s
            }
        };
        let exts = settings.photos_extensions.clone();
        let roots: Vec<PathBuf> = settings
            .photos_folders
            .iter()
            .filter_map(|f| expand_home(f))
            .collect();
        let walk = scan::walk(
            &roots,
            Rules {
                skip_photo_libraries: true,
            },
            |_| {
                if out.cancelled() {
                    Flow::Stop
                } else {
                    Flow::Continue
                }
            },
            |path, info| {
                let ext_ok = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)));
                if !ext_ok {
                    return Flow::Continue;
                }
                if out.cancelled() {
                    return Flow::Stop;
                }
                out.count("files_seen", 1);
                let stamp = FileStamp {
                    path_hash: scan::path_hash(salt, path),
                    size: info.size,
                    mtime_ms: info.mtime_ms,
                };
                if out.file_unchanged(stamp) {
                    out.count("files_unchanged", 1);
                    return Flow::Continue;
                }
                let Ok(f) = File::open(path) else {
                    // Not cached: retried next scan.
                    out.count("open_errors", 1);
                    return Flow::Continue;
                };
                let mut r = Capped::new(f, HARD_CAP);
                let res = read_meta(&mut r, info.size);
                out.count("bytes_read", r.read);
                match res {
                    Ok(m) => match (m.taken, m.shot_id()) {
                        (Some(taken), Some(id)) => {
                            let (t, known) = capture_time(&m, taken);
                            out.item(id, t, known, attrs(&m));
                            out.count("parsed", 1);
                        }
                        _ => out.count("skipped_no_date", 1),
                    },
                    Err(MetaError::Io(_)) => {
                        out.count("read_errors", 1);
                        return Flow::Continue; // retried next scan
                    }
                    Err(MetaError::NoExif) => out.count("skipped_no_exif", 1),
                    Err(MetaError::Unsupported) => out.count("skipped_unsupported", 1),
                }
                out.file_done(stamp);
                Flow::Continue
            },
        );
        out.count("dir_errors", walk.errors);
        if !walk.stopped {
            out.prune_unseen_files();
        }
    }

    fn metrics(&self) -> Vec<MetricDef> {
        let m = |id, title, key| MetricDef {
            id,
            title,
            unit: Unit::Count, agg: Agg::Sum,
            key,
        };
        vec![
            m("shots", "Photos taken", None),
            m("shots_by_camera", "Photos by camera", Some("camera make + model")),
            m("shots_by_lens", "Photos by lens", Some("lens model")),
            m("shots_by_hour", "Photos by hour of day", Some("local hour 00..23")),
            m(
                "shots_by_focal_length",
                "Photos by focal length",
                Some("focal length, whole mm"),
            ),
        ]
    }

    fn rollup(&self, day: &DayInput<'_>) -> Vec<MetricValue> {
        if day.items.is_empty() {
            return Vec::new();
        }
        let mut groups: BTreeMap<(&'static str, String), f64> = BTreeMap::new();
        for it in day.items {
            let lens = it.attr_str("lens").unwrap_or(UNKNOWN).to_string();
            let focal = it
                .attr_f64("focal_mm")
                .map(|f| format!("{}", f.round() as i64))
                .unwrap_or_else(|| UNKNOWN.into());
            for k in [
                ("shots_by_camera", camera_of(it)),
                ("shots_by_lens", lens),
                ("shots_by_hour", format!("{:02}", it.local_hour())),
                ("shots_by_focal_length", focal),
            ] {
                *groups.entry(k).or_default() += 1.0;
            }
        }
        let mut out = vec![MetricValue::new("shots", "", day.items.len() as f64)];
        out.extend(
            groups
                .into_iter()
                .map(|((m, k), v)| MetricValue::new(m, k, v)),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_labels() {
        assert_eq!(camera_label(Some("SONY"), Some("ILCE-7M4")), "SONY ILCE-7M4");
        assert_eq!(camera_label(Some("Canon"), Some("Canon EOS R5")), "Canon EOS R5");
        assert_eq!(
            camera_label(Some("NIKON CORPORATION"), Some("NIKON Z 6")),
            "NIKON Z 6"
        );
        assert_eq!(camera_label(None, None), UNKNOWN);
    }

    #[test]
    fn capture_time_with_and_without_offset() {
        let m = PhotoMeta {
            offset_min: Some(-300),
            subsec: Some("5".into()),
            ..Default::default()
        };
        let taken = NaiveDateTime::parse_from_str("2026-11-14 23:30:00", "%Y-%m-%d %H:%M:%S")
            .unwrap();
        let (t, known) = capture_time(&m, taken);
        assert!(known);
        assert_eq!(t.local_date().to_string(), "2026-11-14");
        assert_eq!(t.to_rfc3339(), "2026-11-14T23:30:00-05:00");
        assert_eq!(t.local_ms() % 1000, 500);
        let (t, known) = capture_time(&PhotoMeta::default(), taken);
        assert!(!known);
        assert_eq!(t.local_date().to_string(), "2026-11-14");
    }
}
