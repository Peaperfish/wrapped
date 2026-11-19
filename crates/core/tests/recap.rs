//! Recaps: period boundaries, travel, the thin-data and December rules,
//! skipped slides, changes against the previous period, the time-of-day label
//! and notification timing.

mod common;

use chrono::{NaiveDate, NaiveDateTime};
use common::*;
use wrapped_core::Store;
use wrapped_core::recap::{
    self, DayLabel, Period, PeriodKind, Recap, RecapEntry, Slide, day_label, due, latest_ready,
    notify_baseline, ready_at,
};
use wrapped_core::stats::Stats;

fn put(s: &Store, date: &str, collector: &str, metric: &str, key: &str, value: f64) {
    s.conn()
        .execute(
            "INSERT OR REPLACE INTO daily_metrics(date, collector, metric, key, value) VALUES (?1,?2,?3,?4,?5)",
            (date, collector, metric, key, value),
        )
        .unwrap();
}

fn screen(s: &Store, date: &str, secs: f64) {
    put(s, date, "app_usage", "screen_time_secs", "", secs);
    put(s, date, "app_usage", "app_secs", "com.apple.dt.Xcode", secs);
}

fn wk(date: &str) -> Period {
    Period::containing(PeriodKind::Week, d(date))
}

fn list(s: &Store, today: &str) -> Vec<RecapEntry> {
    let all = collectors_all();
    let set = settings(s);
    let st = Stats::new(s, &all, &set);
    recap::list(&st, d(today)).unwrap()
}

fn build(s: &Store, p: Period, today: &str) -> Recap {
    let all = collectors_all();
    let set = settings(s);
    let st = Stats::new(s, &all, &set);
    recap::build(&st, p, d(today)).unwrap().unwrap()
}

fn kinds(r: &Recap) -> Vec<&'static str> {
    r.slides.iter().map(Slide::kind).collect()
}

fn ids(v: &[RecapEntry]) -> Vec<String> {
    v.iter().map(|e| e.period.id()).collect()
}

fn dt(s: &str) -> NaiveDateTime {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
}

// ---------- periods ----------

#[test]
fn week_crossing_a_month_and_a_year_end() {
    let p = wk("2026-01-01");
    assert_eq!(p.start, d("2025-12-29"));
    assert_eq!(p.end(), d("2026-01-04"));
    assert_eq!(p.id(), "week:2025-12-29");
    assert_eq!(p.label(), "29 Dec 2025 – 4 Jan 2026");
    assert_eq!(wk("2026-11-01").label(), "26 Oct – 1 Nov 2026");
    assert_eq!(wk("2026-11-19").label(), "16–22 Nov 2026");
    assert_eq!(p.previous().start, d("2025-12-22"));
    assert_eq!(p.next().start, d("2026-01-05"));
    // Any date inside parses to the same period.
    assert_eq!(Period::parse_id("week:2026-01-04"), Some(p));
    assert_eq!(Period::parse_id("week:2025-12-29"), Some(p));

    let feb = Period::containing(PeriodKind::Month, d("2028-02-15"));
    assert_eq!((feb.start, feb.end()), (d("2028-02-01"), d("2028-02-29")));
    assert_eq!(feb.label(), "February 2028");
    let dec = Period::containing(PeriodKind::Month, d("2025-12-31"));
    assert_eq!(dec.next().start, d("2026-01-01"));
    assert_eq!(dec.previous().start, d("2025-11-01"));
    assert_eq!(Period::parse_id("month:2025-12"), Some(dec));
    let y = Period::containing(PeriodKind::Year, d("2026-07-04"));
    assert_eq!((y.start, y.end()), (d("2026-01-01"), d("2026-12-31")));
    assert_eq!(Period::parse_id("year:2026"), Some(y));
    assert_eq!(Period::parse_id("decade:2026"), None);

    // The year-end week's data stays in that week, split across both years' months.
    let s = store_with(ALL);
    screen(&s, "2025-12-30", 3600.0);
    screen(&s, "2026-01-02", 7200.0);
    screen(&s, "2026-01-05", 600.0); // next week
    let r = build(&s, p, "2026-01-10");
    assert_eq!(r.active_days, 2);
    match &r.slides[1] {
        Slide::ScreenTime { total_secs, .. } => assert_eq!(*total_secs, 10_800.0),
        other => panic!("{other:?}"),
    }
    assert!(ids(&list(&s, "2026-01-10")).contains(&"week:2025-12-29".to_string()));
}

#[test]
fn utc_offset_change_mid_week_uses_each_moments_local_day() {
    // Travel from UTC+8 to UTC-5 on Sunday 15 Nov. A session on Sunday
    // 20:00-21:00 in UTC-5 is Monday 09:00 in UTC+8 (and Monday 01:00 UTC):
    // it belongs to Sunday, so to the week of 9 Nov.
    let mut s = store_with(&["app_usage"]);
    let mut p = Pending::default();
    p.session(
        "app_usage",
        "com.a",
        at(d("2026-11-12"), 10, 0, 480),
        at(d("2026-11-12"), 11, 0, 480),
    );
    p.session(
        "app_usage",
        "com.a",
        at(d("2026-11-15"), 20, 0, -300),
        at(d("2026-11-15"), 21, 0, -300),
    );
    p.session(
        "app_usage",
        "com.b",
        at(d("2026-11-16"), 9, 0, -300),
        at(d("2026-11-16"), 9, 30, -300),
    );
    p.flush(&mut s, at(d("2026-11-17"), 12, 0, -300));
    let r = build(&s, wk("2026-11-09"), "2026-11-20");
    match &r.slides[1] {
        Slide::ScreenTime {
            total_secs,
            screen_days,
            ..
        } => {
            assert_eq!((*total_secs, *screen_days), (7200.0, 2));
        }
        other => panic!("{other:?}"),
    }
    let next = build(&s, wk("2026-11-16"), "2026-11-23");
    match &next.slides[1] {
        Slide::ScreenTime { total_secs, .. } => assert_eq!(*total_secs, 1800.0),
        other => panic!("{other:?}"),
    }
}

// ---------- availability ----------

#[test]
fn thin_periods_get_no_recap() {
    let s = store_with(ALL);
    // Week of 2 Nov: one day. Week of 9 Nov: two days.
    screen(&s, "2026-11-04", 3600.0);
    screen(&s, "2026-11-10", 3600.0);
    screen(&s, "2026-11-11", 3600.0);
    let l = list(&s, "2026-11-19");
    assert_eq!(ids(&l), vec!["week:2026-11-09"]);
    assert_eq!(l[0].active_days, 2);
    assert_eq!(l[0].screen_secs, Some(7200.0));
    // The running week never shows, however much data it has.
    for day in ["2026-11-16", "2026-11-17", "2026-11-18"] {
        screen(&s, day, 60.0);
    }
    assert!(!ids(&list(&s, "2026-11-19")).contains(&"week:2026-11-16".to_string()));
    assert!(ids(&list(&s, "2026-11-23")).contains(&"week:2026-11-16".to_string()));

    // Months need 7 days: October has 6, September 7.
    let s = store_with(ALL);
    for day in 1..=6 {
        screen(&s, &format!("2026-10-{day:02}"), 60.0);
    }
    for day in 1..=7 {
        screen(&s, &format!("2026-09-{:02}", day * 2), 60.0);
    }
    let months: Vec<String> = ids(&list(&s, "2026-11-19"))
        .into_iter()
        .filter(|i| i.starts_with("month"))
        .collect();
    assert_eq!(months, vec!["month:2026-09-01"]);

    // Years need 30 days: 2024 has 29, 2025 has 30.
    let s = store_with(ALL);
    let mut day = d("2024-03-01");
    for _ in 0..29 {
        screen(&s, &day.to_string(), 60.0);
        day += chrono::Duration::days(3);
    }
    let mut day = d("2025-03-01");
    for _ in 0..30 {
        screen(&s, &day.to_string(), 60.0);
        day += chrono::Duration::days(3);
    }
    let years: Vec<String> = ids(&list(&s, "2026-06-01"))
        .into_iter()
        .filter(|i| i.starts_with("year"))
        .collect();
    assert_eq!(years, vec!["year:2025-01-01"]);
}

#[test]
fn any_collector_that_is_on_makes_an_active_day() {
    let s = store_with(&["git"]);
    put(&s, "2026-11-10", "git", "commits", "", 3.0);
    put(&s, "2026-11-12", "git", "commits", "", 1.0);
    // A collector that is off does not count.
    put(&s, "2026-11-03", "photos", "shots", "", 5.0);
    put(&s, "2026-11-04", "photos", "shots", "", 5.0);
    let l = list(&s, "2026-11-19");
    assert_eq!(ids(&l), vec!["week:2026-11-09"]);
    assert_eq!(l[0].screen_secs, None);
}

#[test]
fn current_year_gets_a_recap_from_1_december() {
    let s = store_with(ALL);
    let mut day = d("2026-01-05");
    for _ in 0..40 {
        screen(&s, &day.to_string(), 3600.0);
        day += chrono::Duration::days(7);
    }
    let years = |today: &str| -> Vec<RecapEntry> {
        list(&s, today)
            .into_iter()
            .filter(|e| e.period.kind == PeriodKind::Year)
            .collect()
    };
    assert!(years("2026-11-30").is_empty());
    let dec1 = years("2026-12-01");
    assert_eq!(ids(&dec1), vec!["year:2026-01-01"]);
    assert!(dec1[0].in_progress);
    assert_eq!(
        (dec1[0].range.from, dec1[0].range.to),
        (d("2026-01-01"), d("2026-12-01"))
    );
    let r = build(&s, dec1[0].period, "2026-12-01");
    assert!(r.in_progress && r.available);
    assert_eq!(r.range.to, d("2026-12-01"));
    // Compared with the same days of 2025.
    assert_eq!(
        (r.previous.from, r.previous.to),
        (d("2025-01-01"), d("2025-12-01"))
    );
    let dec31 = years("2026-12-31");
    assert!(dec31[0].in_progress);
    let jan = years("2027-01-01");
    assert!(!jan[0].in_progress);
    assert_eq!(jan[0].range.to, d("2026-12-31"));
}

// ---------- slides ----------

#[test]
fn slides_of_collectors_that_are_off_or_empty_are_skipped() {
    let s = store_with(&["app_usage", "git", "photos", "disk_memory"]);
    for day in ["2026-11-10", "2026-11-11", "2026-11-12"] {
        screen(&s, day, 3600.0);
        put(&s, day, "app_usage", "active_secs_by_hour", "10", 3600.0);
        // ai_usage is off: kept data, no slide.
        put(&s, day, "ai_usage", "tokens", "", 1e6);
    }
    put(&s, "2026-11-11", "git", "commits", "", 4.0);
    put(&s, "2026-11-11", "git", "commits_by_repo", "wrapped", 4.0);
    put(&s, "2026-11-11", "git", "commits_by_hour", "15", 4.0);
    // photos is on but has nothing this week; disk_memory wrote nothing.
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    assert_eq!(
        kinds(&r),
        vec![
            "intro",
            "screen_time",
            "top_apps",
            "time_of_day",
            "busiest_day",
            "records",
            "git",
            "summary"
        ]
    );
    match r.slides.last().unwrap() {
        Slide::Summary { items } => {
            let ids: Vec<&str> = items.iter().map(|h| h.id).collect();
            assert_eq!(
                ids,
                vec!["screen_time", "top_app", "commits", "streak", "active_days"]
            );
            assert_eq!(items[1].text.as_deref(), Some("com.apple.dt.Xcode"));
        }
        other => panic!("{other:?}"),
    }
    match &r.slides[6] {
        Slide::Git {
            top_repo,
            busiest_hour,
            ..
        } => {
            assert_eq!(top_repo.as_ref().unwrap().key, "wrapped");
            assert_eq!(*busiest_hour, Some(15));
        }
        other => panic!("{other:?}"),
    }

    // Only git on: no screen-time slides at all, intro without screen time.
    let s = store_with(&["git"]);
    put(&s, "2026-11-10", "git", "commits", "", 1.0);
    put(&s, "2026-11-11", "git", "commits", "", 1.0);
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    assert_eq!(kinds(&r), vec!["intro", "git", "summary"]);
    assert!(matches!(
        r.slides[0],
        Slide::Intro {
            screen_secs: None,
            ..
        }
    ));
}

#[test]
fn photos_and_ai_slides_skip_unknown_keys() {
    let s = store_with(&["photos", "ai_usage"]);
    for day in ["2026-11-10", "2026-11-11"] {
        put(&s, day, "photos", "shots", "", 10.0);
        put(&s, day, "photos", "shots_by_camera", "(unknown)", 6.0);
        put(&s, day, "photos", "shots_by_camera", "SONY ILCE-7M4", 4.0);
        put(&s, day, "photos", "shots_by_lens", "FE 35mm F1.8", 10.0);
        put(&s, day, "photos", "shots_by_focal_length", "35", 10.0);
        put(&s, day, "ai_usage", "tokens", "", 300.0);
        put(&s, day, "ai_usage", "tokens_by_kind", "input", 100.0);
        put(&s, day, "ai_usage", "tokens_by_kind", "output", 200.0);
        put(&s, day, "ai_usage", "tokens_by_tool", "claude", 300.0);
        put(
            &s,
            day,
            "ai_usage",
            "tokens_by_model",
            "claude-opus-5-5",
            300.0,
        );
        put(&s, day, "ai_usage", "tokens_by_project", "wrapped", 300.0);
    }
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    assert_eq!(kinds(&r), vec!["intro", "photos", "ai", "summary"]);
    match &r.slides[1] {
        Slide::Photos {
            count,
            top_camera,
            top_focal,
            ..
        } => {
            assert_eq!(*count, 20.0);
            assert_eq!(
                top_camera.as_ref().unwrap().key,
                wrapped_core::collectors::photos::friendly_camera("SONY ILCE-7M4")
            );
            assert_eq!(top_focal.as_ref().unwrap().key, "35");
        }
        other => panic!("{other:?}"),
    }
    match &r.slides[2] {
        Slide::Ai {
            tokens,
            input,
            output,
            top_tool,
            ..
        } => {
            assert_eq!((*tokens, *input, *output), (600.0, 200.0, 400.0));
            assert_eq!(top_tool.as_ref().unwrap().key, "claude");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn records_and_busiest_day() {
    let mut s = store_with(&["app_usage"]);
    let mut p = Pending::default();
    let day = |x: &str| d(x);
    p.session(
        "app_usage",
        "com.a",
        at(day("2026-11-10"), 9, 0, 60),
        at(day("2026-11-10"), 12, 0, 60),
    );
    p.session(
        "app_usage",
        "com.b",
        at(day("2026-11-11"), 23, 0, 60),
        at(day("2026-11-12"), 1, 30, 60),
    );
    p.session(
        "app_usage",
        "com.a",
        at(day("2026-11-12"), 10, 0, 60),
        at(day("2026-11-12"), 10, 30, 60),
    );
    p.flush(&mut s, at(day("2026-11-13"), 9, 0, 60));
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    let rec = r.slides.iter().find(|x| x.kind() == "records").unwrap();
    match rec {
        Slide::Records {
            longest_session,
            latest_night,
            streak,
        } => {
            let l = longest_session.as_ref().unwrap();
            assert_eq!((l.app.as_str(), l.secs), ("com.a", 10_800.0));
            assert_eq!(latest_night.as_ref().unwrap().clock_min, 25 * 60 + 30);
            assert_eq!(streak.unwrap().days, 3);
        }
        other => panic!("{other:?}"),
    }
    let busiest = r.slides.iter().find(|x| x.kind() == "busiest_day").unwrap();
    assert_eq!(
        *busiest,
        Slide::BusiestDay {
            date: d("2026-11-10"),
            secs: 10_800.0
        }
    );
}

// ---------- changes ----------

#[test]
fn change_against_the_previous_period_and_the_thin_rule() {
    let s = store_with(ALL);
    for day in ["2026-11-02", "2026-11-03", "2026-11-04"] {
        screen(&s, day, 3600.0);
    }
    for day in ["2026-11-09", "2026-11-10", "2026-11-11", "2026-11-12"] {
        screen(&s, day, 3600.0);
    }
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    assert!(r.comparable);
    assert_eq!(
        (r.previous.from, r.previous.to),
        (d("2026-11-02"), d("2026-11-08"))
    );
    match &r.slides[1] {
        Slide::ScreenTime { change_pct, .. } => {
            let pct = change_pct.unwrap();
            assert!((pct - 33.333).abs() < 0.01, "{pct}");
        }
        other => panic!("{other:?}"),
    }
    // A month whose previous month has 3 days against 7: too thin, every change "—".
    let s = store_with(ALL);
    for day in 1..=3 {
        screen(&s, &format!("2026-09-{day:02}"), 3600.0);
    }
    for day in 1..=7 {
        screen(&s, &format!("2026-10-{day:02}"), 3600.0);
    }
    let r = build(
        &s,
        Period::containing(PeriodKind::Month, d("2026-10-15")),
        "2026-11-19",
    );
    assert!(!r.comparable);
    match &r.slides[1] {
        Slide::ScreenTime { change_pct, .. } => assert_eq!(*change_pct, None),
        other => panic!("{other:?}"),
    }
    // Exactly half as many days is still comparable.
    assert!(!recap::too_thin(8, 4));
    assert!(recap::too_thin(9, 4));
}

// ---------- labels ----------

#[test]
fn time_of_day_label_thresholds() {
    let mut h = [0.0; 24];
    h[14] = 100.0;
    assert_eq!(day_label(&h).0, DayLabel::Daytime);
    // 25 % between 22:00 and 05:00: night owl.
    let mut h = [0.0; 24];
    h[14] = 75.0;
    h[23] = 15.0;
    h[2] = 10.0;
    assert_eq!(day_label(&h).0, DayLabel::NightOwl);
    h[14] = 76.0;
    assert_eq!(day_label(&h).0, DayLabel::Daytime);
    // 40 % between 05:00 and 12:00: early bird.
    let mut h = [0.0; 24];
    h[7] = 40.0;
    h[15] = 60.0;
    assert_eq!(day_label(&h).0, DayLabel::EarlyBird);
    h[15] = 61.0;
    assert_eq!(day_label(&h).0, DayLabel::Daytime);
    // Night owl wins over early bird.
    let mut h = [0.0; 24];
    h[6] = 50.0;
    h[1] = 50.0;
    assert_eq!(day_label(&h).0, DayLabel::NightOwl);
    assert_eq!(day_label(&[0.0; 24]).0, DayLabel::Daytime);
}

// ---------- notifications ----------

#[test]
fn notification_timing_newest_period_only() {
    let s = store_with(ALL);
    for day in ["2026-11-03", "2026-11-04", "2026-11-10", "2026-11-11"] {
        screen(&s, day, 3600.0);
    }
    let avail = |today: &str| list(&s, today);
    // Monday 16 Nov before 09:00: last week is not announced yet.
    assert_eq!(
        latest_ready(PeriodKind::Week, dt("2026-11-16 08:59")),
        wk("2026-11-02")
    );
    assert_eq!(ready_at(wk("2026-11-09")), dt("2026-11-16 09:00"));
    let l = avail("2026-11-16");
    let last = Some(wk("2026-11-02"));
    assert_eq!(
        due(PeriodKind::Week, dt("2026-11-16 08:59"), last, &l),
        None
    );
    assert_eq!(
        due(PeriodKind::Week, dt("2026-11-16 09:00"), last, &l),
        Some(wk("2026-11-09"))
    );
    // Already announced.
    assert_eq!(
        due(
            PeriodKind::Week,
            dt("2026-11-16 10:00"),
            Some(wk("2026-11-09")),
            &l
        ),
        None
    );
    // Asleep for weeks: only the newest ready week, and only if it has a recap.
    let l = avail("2026-11-30");
    assert_eq!(
        due(
            PeriodKind::Week,
            dt("2026-11-30 12:00"),
            Some(wk("2026-10-26")),
            &l
        ),
        None
    );
    let l = avail("2026-11-17");
    assert_eq!(
        due(
            PeriodKind::Week,
            dt("2026-11-17 12:00"),
            Some(wk("2026-10-26")),
            &l
        ),
        Some(wk("2026-11-09"))
    );
    // First time on: the baseline is the week before the newest ready one.
    assert_eq!(
        notify_baseline(PeriodKind::Week, dt("2026-11-18 12:00")),
        wk("2026-11-02")
    );
    // Years: ready on 1 December at 09:00 (the in-progress recap).
    let y26 = Period::containing(PeriodKind::Year, d("2026-06-01"));
    assert_eq!(ready_at(y26), dt("2026-12-01 09:00"));
    assert_eq!(latest_ready(PeriodKind::Year, dt("2026-12-01 09:00")), y26);
    assert_eq!(
        latest_ready(PeriodKind::Year, dt("2026-11-30 23:00")),
        y26.previous()
    );
    assert_eq!(latest_ready(PeriodKind::Year, dt("2027-01-01 10:00")), y26);
    // Months: the 1st at 09:00.
    let oct = Period::containing(PeriodKind::Month, d("2026-10-15"));
    assert_eq!(ready_at(oct), dt("2026-11-01 09:00"));
}

#[test]
fn notification_text_has_one_number_and_the_top_app() {
    let s = store_with(ALL);
    for day in ["2026-11-10", "2026-11-11"] {
        screen(&s, day, 16.0 * 3600.0);
    }
    let r = build(&s, wk("2026-11-09"), "2026-11-19");
    let text = recap::notification_text(&r, |id| {
        if id == "com.apple.dt.Xcode" {
            "Xcode".into()
        } else {
            id.into()
        }
    });
    assert_eq!(text, "Your week is ready: 32 h screen time, top app Xcode");
}

#[test]
fn recap_json_has_a_kind_per_slide() {
    let s = store_with(ALL);
    for day in ["2026-11-10", "2026-11-11"] {
        screen(&s, day, 3600.0);
    }
    let v = build(&s, wk("2026-11-09"), "2026-11-19").to_json();
    assert_eq!(v["id"], "week:2026-11-09");
    assert_eq!(v["label"], "9–15 Nov 2026");
    let kinds: Vec<&str> = v["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.iter().all(|k| recap::SLIDE_KINDS.contains(k)));
    let _: NaiveDate = v["range"]["from"].as_str().unwrap().parse().unwrap();
}

#[test]
fn notify_check_baseline_once_and_no_burst() {
    use wrapped_core::recap::notify;
    let s = store_with(ALL);
    let all = collectors_all();
    for day in [
        "2026-11-03",
        "2026-11-04",
        "2026-11-10",
        "2026-11-11",
        "2026-11-17",
        "2026-11-18",
    ] {
        screen(&s, day, 3600.0);
    }
    let local = |date: &str, h: i64| at(d(date), h, 0, 60);
    let run = |now| notify::check(&s, &settings(&s), &all, now, |id: &str| id.to_string()).unwrap();
    // Everything off: nothing, and nothing stored.
    assert!(run(local("2026-11-18", 12)).is_empty());
    assert_eq!(
        s.internal_value(&notify::last_key(PeriodKind::Week))
            .unwrap(),
        None
    );

    // Turned on on Wednesday 18 Nov: the newest ready week (9 Nov) once, never the older ones.
    s.set_setting("recaps.notify.week", "true").unwrap();
    let n = run(local("2026-11-18", 12));
    assert_eq!(n.len(), 1);
    assert_eq!(n[0].period, wk("2026-11-09"));
    assert!(
        n[0].body.starts_with("Your week is ready: 2 h screen time"),
        "{}",
        n[0].body
    );
    assert!(run(local("2026-11-18", 13)).is_empty());
    // Monday 23 Nov before 09:00: nothing; from 09:00: the week of 16 Nov.
    assert!(run(local("2026-11-23", 8)).is_empty());
    assert_eq!(run(local("2026-11-23", 9))[0].period, wk("2026-11-16"));
    // Asleep for two weeks with data: only the newest week.
    for day in ["2026-11-24", "2026-11-25", "2026-12-01", "2026-12-02"] {
        screen(&s, day, 600.0);
    }
    let n = run(local("2026-12-08", 10));
    assert_eq!(
        n.iter().map(|x| x.period).collect::<Vec<_>>(),
        vec![wk("2026-11-30")]
    );
    // Months and years stay quiet while off.
    assert_eq!(
        s.internal_value(&notify::last_key(PeriodKind::Month))
            .unwrap(),
        None
    );
}
