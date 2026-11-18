//! Requests from the dashboard / CLI (`Engine::handle_request`) and the
//! first-launch setup mode, against a real engine and an in-memory store.

use std::sync::Arc;

use serde_json::{Value, json};
use wrapped_core::ctl::{self, Reply, Request};
use wrapped_core::{Engine, ManualClock, Store, SystemEvent, collectors};

fn engine(store: Store) -> (Engine, ManualClock) {
    let c = ManualClock::at_local(2026, 11, 16, 9, 0, 0, 480);
    let mut e = Engine::new(store, Arc::new(c.clone()), collectors::builtin_hermetic()).unwrap();
    e.set_auto_scans(false);
    (e, c)
}

fn req(e: &mut Engine, cmd: &str, args: Value) -> Reply {
    e.handle_request(&Request::new(cmd, args))
}

fn ok(r: Reply) -> Value {
    match r {
        Reply::Ok(v) => v,
        Reply::Err { code, error } => panic!("expected ok, got {code}: {error}"),
    }
}

fn err_code(r: Reply) -> String {
    match r {
        Reply::Err { code, .. } => code,
        Reply::Ok(v) => panic!("expected an error, got {v}"),
    }
}

fn app(e: &mut Engine, id: &str) {
    e.handle(SystemEvent::FrontApp {
        bundle_id: id.into(),
        name: id.into(),
    })
    .unwrap();
}

#[test]
fn set_settings_applies_at_once_and_rejects_bad_input_whole() {
    let (mut e, _) = engine(Store::open_in_memory().unwrap());
    ok(req(
        &mut e,
        ctl::CMD_SET_SETTINGS,
        json!({"settings": {"exclude.apps": ["com.example.secret"], "idle_threshold_secs": 240}}),
    ));
    assert!(e.settings().is_excluded("com.example.secret"));
    assert_eq!(e.settings().idle_threshold_secs, 240);
    // One bad value: nothing from the same request is stored.
    let r = req(
        &mut e,
        ctl::CMD_SET_SETTINGS,
        json!({"settings": {"idle_threshold_secs": 5, "exclude.apps": []}}),
    );
    assert_eq!(err_code(r), "invalid");
    assert_eq!(e.settings().idle_threshold_secs, 240);
    assert!(e.settings().is_excluded("com.example.secret"));
    assert_eq!(err_code(req(&mut e, "nope", Value::Null)), "invalid");
    assert_eq!(
        err_code(req(
            &mut e,
            ctl::CMD_SET_SETTINGS,
            json!({"settings": {"collector.nothing.enabled": true}})
        )),
        "invalid"
    );
}

#[test]
fn set_collector_and_pause() {
    let (mut e, _) = engine(Store::open_in_memory().unwrap());
    assert!(e.running_collectors().contains(&"app_usage"));
    ok(req(
        &mut e,
        ctl::CMD_SET_COLLECTOR,
        json!({"name": "app_usage", "enabled": false, "delete_data": false}),
    ));
    assert!(!e.running_collectors().contains(&"app_usage"));
    ok(req(
        &mut e,
        ctl::CMD_SET_COLLECTOR,
        json!({"name": "app_usage", "enabled": true}),
    ));
    assert!(e.running_collectors().contains(&"app_usage"));
    ok(req(&mut e, ctl::CMD_PAUSE, json!({"mode": "hour"})));
    assert!(e.is_paused());
    ok(req(&mut e, ctl::CMD_PAUSE, json!({"mode": "off"})));
    assert!(!e.is_paused());
    assert_eq!(
        err_code(req(&mut e, ctl::CMD_PAUSE, json!({"mode": "x"}))),
        "invalid"
    );
}

#[test]
fn delete_all_needs_the_typed_word_and_keeps_settings() {
    let (mut e, c) = engine(Store::open_in_memory().unwrap());
    ok(req(
        &mut e,
        ctl::CMD_SET_SETTINGS,
        json!({"settings": {"exclude.apps": ["x.y"]}}),
    ));
    app(&mut e, "com.apple.Safari");
    c.advance_ms(600_000);
    app(&mut e, "com.apple.Mail");
    e.flush().unwrap();
    assert!(e.store().has_any_data().unwrap());
    assert_eq!(
        err_code(req(&mut e, ctl::CMD_DELETE_ALL, json!({"confirm": "yes"}))),
        "invalid"
    );
    assert!(e.store().has_any_data().unwrap());
    ok(req(
        &mut e,
        ctl::CMD_DELETE_ALL,
        json!({"confirm": "delete"}),
    ));
    assert!(!e.store().has_any_data().unwrap());
    assert!(e.settings().is_excluded("x.y"));
}

#[test]
fn fresh_database_starts_in_setup_mode_and_records_nothing() {
    let (mut e, c) = engine(Store::open_in_memory().unwrap());
    assert!(e.begin_onboarding().unwrap());
    assert!(e.is_setup_mode());
    app(&mut e, "com.apple.Safari");
    c.advance_ms(900_000);
    app(&mut e, "com.apple.Mail");
    e.flush().unwrap();
    assert!(
        !e.store().has_any_data().unwrap(),
        "nothing recorded before setup"
    );
    // Scans are refused too.
    let v = ok(req(&mut e, ctl::CMD_SCAN, json!({"what": "git"})));
    assert!(v["started"].as_array().unwrap().is_empty());
    let st = ok(req(&mut e, ctl::CMD_STATUS, Value::Null));
    assert_eq!(st["setup"], json!(true));

    ok(req(
        &mut e,
        ctl::CMD_FINISH_SETUP,
        json!({
            "settings": {"git.folders": ["~/code"]},
            "collectors": {"app_usage": true, "active_idle": true, "git": false,
                           "photos": false, "disk_memory": false}
        }),
    ));
    assert!(!e.is_setup_mode());
    assert_eq!(e.settings().onboarding_done, Some(true));
    assert!(!e.running_collectors().contains(&"git"));
    // Finishing twice is refused.
    assert_eq!(
        err_code(req(&mut e, ctl::CMD_FINISH_SETUP, json!({}))),
        "invalid"
    );
    // Recording starts now.
    app(&mut e, "com.apple.Safari");
    c.advance_ms(600_000);
    app(&mut e, "com.apple.Mail");
    e.flush().unwrap();
    assert!(e.store().has_any_data().unwrap());
}

#[test]
fn existing_install_skips_setup_and_keeps_its_settings() {
    let store = Store::open_in_memory().unwrap();
    store.set_setting("exclude.apps", r#"["keep.me"]"#).unwrap();
    store.set_setting("collector.git.enabled", "true").unwrap();
    store
        .conn()
        .execute(
            "INSERT INTO daily_metrics(date, collector, metric, key, value)
             VALUES ('2026-11-01', 'app_usage', 'screen_secs', '', 60)",
            [],
        )
        .unwrap();
    let before = store.load_settings().unwrap().0;
    let (mut e, _) = engine(store);
    assert!(!e.begin_onboarding().unwrap());
    let after = e.settings().clone();
    assert_eq!(after.onboarding_done, Some(true));
    // Everything else is exactly as it was.
    let mut expected = before;
    expected.onboarding_done = Some(true);
    assert_eq!(after, expected);
    assert!(e.running_collectors().contains(&"git"));
}
