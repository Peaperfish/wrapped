//! `now_snapshot` returns the JSON `NowSnapshotJson` describes.

use std::sync::Mutex;

use wrapped_dashboard::data;
use wrapped_dashboard::types::NowSnapshotJson;

#[test]
fn now_snapshot_matches_its_type_and_rates_arrive_on_the_second_call() {
    let sampler = Mutex::new(None);
    let first = data::now_snapshot(&sampler);
    let typed: NowSnapshotJson = serde_json::from_value(first.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), first);
    // First call: no rates yet.
    assert!(first["cpu_percent"].is_null());
    assert!(first["disk_read_per_sec"].is_null());

    std::thread::sleep(std::time::Duration::from_millis(250));
    let second = data::now_snapshot(&sampler);
    let typed: NowSnapshotJson = serde_json::from_value(second).unwrap();
    #[cfg(target_os = "macos")]
    {
        assert!(
            typed
                .cpu_percent
                .is_some_and(|p| (0.0..=100.0).contains(&p))
        );
        assert!(typed.net_down_per_sec.is_some_and(|r| r >= 0.0));
        assert!(typed.mem_total_bytes.is_some());
    }
    let _ = typed;
}
