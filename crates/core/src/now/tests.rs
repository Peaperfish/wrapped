use std::collections::VecDeque;

use super::*;

/// Replays prepared readings.
struct Fake(VecDeque<RawNow>);

impl NowSource for Fake {
    fn chip(&mut self) -> Option<String> {
        Some("Test chip".into())
    }
    fn read(&mut self) -> RawNow {
        self.0.pop_front().expect("a prepared reading")
    }
}

fn sampler(readings: Vec<RawNow>) -> NowSampler {
    NowSampler::with_source(Box::new(Fake(readings.into())))
}

fn core(busy: u32, idle: u32, kind: Option<CoreKind>) -> RawCore {
    RawCore { busy, idle, kind }
}

fn at(mono_ms: u64) -> RawNow {
    RawNow {
        mono_ms,
        ..RawNow::default()
    }
}

#[test]
fn byte_rate_maths() {
    assert_eq!(byte_rate(1_000, 3_000, 2.0), Some(1_000.0));
    assert_eq!(byte_rate(5, 5, 1.0), Some(0.0));
    // Counter went backwards (reset / reboot): no rate, never negative.
    assert_eq!(byte_rate(3_000, 10, 1.0), None);
    // Empty interval.
    assert_eq!(byte_rate(0, 10, 0.0), None);
    // A jump no hardware can make is a glitch, not a spike.
    assert_eq!(byte_rate(0, u64::MAX, 1.0), None);
    assert_eq!(byte_rate(0, 1_000_000_000_000, 1.0), None);
}

#[test]
fn core_percent_wraps_and_handles_idle_gap() {
    let a = core(10, 90, None);
    assert_eq!(core_percent(&a, &core(60, 140, None)), Some(50.0));
    // 32-bit tick counters wrap: 10 busy + 90 idle ticks passed.
    let near = core(u32::MAX - 4, u32::MAX - 44, None);
    let wrapped = core(5, 45, None);
    assert_eq!(core_percent(&near, &wrapped), Some(10.0 / 100.0 * 100.0));
    // No ticks between readings: unknown, not 0 or NaN.
    assert_eq!(core_percent(&a, &a), None);
}

#[test]
fn first_call_has_no_rates() {
    let mut raw = at(0);
    raw.cores = Some(vec![core(1, 1, Some(CoreKind::Efficiency))]);
    raw.disk = Some((10, 10));
    raw.net = Some(vec![RawIface {
        id: 1,
        rx: 5,
        tx: 5,
    }]);
    raw.swap_io = Some((1, 1));
    raw.gpu_percent = Some(30.0);
    raw.mem = Some(RawMem {
        used: 4,
        total: 8,
        compressed: Some(1),
        pressure: Some(2),
    });
    let s = sampler(vec![raw]).sample();
    assert_eq!(s.cpu_percent, None);
    assert_eq!(s.cpu_cores.as_ref().unwrap()[0].percent, None);
    assert_eq!(s.disk_read_per_sec, None);
    assert_eq!(s.net_down_per_sec, None);
    assert_eq!(s.swap_in_per_sec, None);
    // Instantaneous values are there from the first call.
    assert_eq!(s.gpu_percent, Some(30.0));
    assert_eq!(s.mem_used_bytes, Some(4));
    assert_eq!(s.mem_pressure.as_deref(), Some("warning"));
    assert_eq!(s.chip.as_deref(), Some("Test chip"));
}

#[test]
fn rates_from_the_delta_between_calls() {
    let mut a = at(0);
    a.cores = Some(vec![
        core(0, 0, Some(CoreKind::Efficiency)),
        core(0, 0, Some(CoreKind::Performance)),
    ]);
    a.disk = Some((1_000, 2_000));
    a.swap_io = Some((0, 0));
    a.net = Some(vec![
        RawIface {
            id: 1,
            rx: 0,
            tx: 0,
        },
        RawIface {
            id: 2,
            rx: 100,
            tx: 0,
        },
    ]);
    let mut b = at(2_000);
    b.cores = Some(vec![
        core(100, 100, Some(CoreKind::Efficiency)),
        core(0, 200, Some(CoreKind::Performance)),
    ]);
    b.disk = Some((5_000, 2_000));
    b.swap_io = Some((4_096, 0));
    b.net = Some(vec![
        RawIface {
            id: 1,
            rx: 4_000,
            tx: 1_000,
        },
        RawIface {
            id: 2,
            rx: 300,
            tx: 0,
        },
    ]);
    let mut sm = sampler(vec![a, b]);
    sm.sample();
    let s = sm.sample();
    assert_eq!(s.cpu_percent, Some(25.0));
    assert_eq!(s.cpu_e_percent, Some(50.0));
    assert_eq!(s.cpu_p_percent, Some(0.0));
    assert_eq!(s.disk_read_per_sec, Some(2_000.0));
    assert_eq!(s.disk_write_per_sec, Some(0.0));
    assert_eq!(s.swap_in_per_sec, Some(2_048.0));
    assert_eq!(s.net_down_per_sec, Some(2_100.0));
    assert_eq!(s.net_up_per_sec, Some(500.0));
}

#[test]
fn counter_reset_gives_none_then_recovers() {
    let mk = |t, read| RawNow {
        disk: Some((read, 0)),
        ..at(t)
    };
    let mut sm = sampler(vec![mk(0, 10_000), mk(1_000, 50), mk(2_000, 1_050)]);
    sm.sample();
    // The counter restarted: no negative number, no huge one.
    assert_eq!(sm.sample().disk_read_per_sec, None);
    // The reset reading became the new baseline.
    assert_eq!(sm.sample().disk_read_per_sec, Some(1_000.0));
}

#[test]
fn network_interface_changes_never_spike() {
    let mk = |t, ifs: Vec<RawIface>| RawNow {
        net: Some(ifs),
        ..at(t)
    };
    let i = |id, rx, tx| RawIface { id, rx, tx };
    let mut sm = sampler(vec![
        mk(0, vec![i(1, 1_000, 0), i(2, 9_000_000_000, 0)]),
        // Interface 2 vanished, 3 appeared with a big counter, 1 reset.
        mk(1_000, vec![i(1, 10, 0), i(3, 7_000_000_000, 0)]),
    ]);
    sm.sample();
    let s = sm.sample();
    assert_eq!(s.net_down_per_sec, Some(0.0));
    assert_eq!(s.net_up_per_sec, Some(0.0));
}

#[test]
fn too_soon_keeps_the_baseline() {
    let mk = |t, read| RawNow {
        disk: Some((read, 0)),
        ..at(t)
    };
    let mut sm = sampler(vec![mk(0, 0), mk(10, 5), mk(1_000, 1_000)]);
    sm.sample();
    assert_eq!(sm.sample().disk_read_per_sec, None);
    // Measured from the first reading, not from the 10 ms one.
    assert_eq!(sm.sample().disk_read_per_sec, Some(1_000.0));
}

#[test]
fn core_count_change_skips_cpu() {
    let mk = |t, n| RawNow {
        cores: Some(vec![core(t as u32, t as u32, None); n]),
        ..at(t)
    };
    let mut sm = sampler(vec![mk(0, 2), mk(1_000, 3)]);
    sm.sample();
    let s = sm.sample();
    assert_eq!(s.cpu_percent, None);
    assert_eq!(s.cpu_cores.as_ref().map(Vec::len), Some(3));
}

#[test]
fn unavailable_fields_stay_none() {
    let mut sm = sampler(vec![at(0), at(1_000)]);
    sm.sample();
    let s = sm.sample();
    assert_eq!(
        s,
        NowSnapshot {
            ts_ms: s.ts_ms,
            chip: Some("Test chip".into()),
            ..NowSnapshot::default()
        }
    );
    let j = s.to_json();
    for k in [
        "cpu_percent",
        "cpu_cores",
        "gpu_percent",
        "mem_used_bytes",
        "temps",
        "fans",
        "net_down_per_sec",
    ] {
        assert!(j[k].is_null(), "{k} should be null");
    }
}

#[test]
fn temperatures_and_fans_pass_through() {
    let mut raw = at(0);
    raw.temps = Some(vec![
        Temp {
            label: "CPU performance cores".into(),
            celsius: 40.0,
        },
        Temp {
            label: "CPU efficiency cores".into(),
            celsius: 50.0,
        },
        Temp {
            label: "GPU".into(),
            celsius: 45.0,
        },
        Temp {
            label: "SSD".into(),
            celsius: 30.0,
        },
    ]);
    raw.fans = Some(vec![Fan {
        name: "Fan 1".into(),
        rpm: 2000.0,
        min_rpm: Some(1200.0),
        max_rpm: None,
    }]);
    let s = sampler(vec![raw]).sample();
    assert_eq!(s.cpu_temp_c, Some(50.0));
    assert_eq!(s.gpu_temp_c, Some(45.0));
    let j = s.to_json();
    assert_eq!(j["fans"][0]["rpm"], 2000.0);
    assert!(j["fans"][0]["max_rpm"].is_null());
    assert_eq!(j["temps"].as_array().unwrap().len(), 4);
}

#[test]
fn non_finite_numbers_become_null() {
    let raw = RawNow {
        gpu_percent: Some(f64::NAN),
        ..at(0)
    };
    let j = sampler(vec![raw]).sample().to_json();
    assert!(j["gpu_percent"].is_null());
}
