# Milestone 8: recaps

DESIGN.md §15 step 8: weekly, monthly and yearly recaps as story slides,
PNG export and optional notifications. Everything stays on this Mac: the
recap is computed from the local database, the PNGs go to a local folder and
the notifications are posted by the local app.

Status: **done** (notification posting and clicking not yet tried in an
installed Wrapped.app; see "Not verified").

## What was built

- `crates/core/src/recap/`: periods, availability, the slides (derived from
  one `Stats::summary_vs` call per recap, no SQL of their own) and the
  notification timing. Core stays free of serde and ts-rs; JSON comes from
  `Recap::to_json` / `RecapEntry::to_json`.
- CLI: `wrapped-collector recap list [--json]`,
  `wrapped-collector recap show week|month|year <date> [--json]` (any date
  inside the period, or `today`), `wrapped-collector recap notify-test
  [week|month|year]`.
- Dashboard: `recap_list` and `recap_get(id)` (async, on a blocking worker
  like `now_snapshot`), typed `RecapEntryJson` / `RecapJson` / `SlideJson`
  (ts-rs, dashboard crate). Tabs are now **Overview | Now | Recaps**; the last
  one is remembered.
- Recaps tab: Years, Months, Weeks, newest first, each with its label, screen
  time (when app usage is on) and active days. Not yet opened: a "New" mark
  (`wrapped.recaps.seen` in localStorage).
- Story player, PNG export, Settings → Recaps and notifications (below).

## Periods and when a recap exists

- Week = Monday–Sunday, month = calendar month, year = calendar year, in
  local dates. Every second, item and byte already sits on the local date its
  rollup gave it (UTC plus the offset stored with it), so a trip mid-week
  moves nothing between weeks. Ids: `week:2026-11-09`, `month:2026-11-01`,
  `year:2026-01-01` (the first day); any date inside parses to the same period.
- **Active day** = a day with any stored metric of a collector that is on (the
  `data_days` of a summary). Not just screen time: on this Mac app usage only
  started on 16 Nov, while git, photos and AI usage go back further.
- A finished period (ended before today) gets a recap with at least **2 active
  days (week), 7 (month), 30 (year)**.
- **December rule**: from 1 December the current year also gets a recap,
  covering 1 January → today, updated every day until 31 December. On 1
  January it becomes the finished year (same id).
- Changes compare with the previous calendar period (for the year in
  progress: the same number of days at the start of the previous year). When
  that period has data on fewer than half as many days (the dashboard's rule)
  every change is "—".

## Slides, in order

A slide is left out when its collector is off or it has nothing in the period.

| Slide | Content | Shown when |
|---|---|---|
| intro | period label, screen time, active days of all days | always |
| screen_time | total, average per day with screen time, change, busiest weekday (highest average; not on weeks) | app usage on, screen time > 0 |
| top_apps | top 5 apps with time and share of screen time | same, any app |
| time_of_day | 24 hourly bars, peak hour, label | any hour > 0 |
| busiest_day | weekday, date, time | screen time > 0 |
| records | longest session (app, length), latest night (clock time, 00:00–05:00 counts as the night before), longest streak of active days | any of the three |
| git | commits, change, top repository, busiest hour + hourly bars | git on, commits > 0 |
| photos | count, change, top camera, top lens, most used focal length ("(unknown)" skipped) | photos on, shots > 0 |
| ai | input + output tokens, the split, change, top tool, model, project | AI usage on, tokens > 0 |
| disk | GB written to internal storage, change | Disk & Memory on, bytes > 0 |
| summary | up to 5 of: screen time, top app, commits, photos, AI tokens, longest streak (> 1 day), disk written, active days (in that priority) | always |

Only what the dashboard already shows: app ids (shown by name), repository
and AI project folder names, camera and lens models. No window titles, files
or paths.

**Time-of-day label** (`recap::day_label`, active seconds by local hour):
night owl when ≥ 25 % falls between 22:00 and 05:00; otherwise early bird
when ≥ 40 % falls between 05:00 and 12:00; otherwise daytime.

## The story player

- A full-window overlay with a 9:16 stage, letterboxed. Segmented progress
  bars; auto-advance every 6 s (stops on the last slide). Click the right /
  left half or ← → for next / previous, Space pauses, press and hold pauses,
  Esc or × closes. ⌘S saves the slide, ⇧⌘S all slides. Page scrolling is
  locked (`scrollLock.ts`); it also pauses while the window is hidden.
- Each slide is **one self-contained SVG** (`lib/recaps/slides.ts`, 1080 ×
  1920 viewBox), hand-drawn with its own layout and background shapes.
  Colours come from the current accent (`lib/recaps/palette.js`): each slide
  kind turns the hue by a fixed amount, the backgrounds are dark (dark theme)
  or pale (light theme), and text, secondary text and highlights are made
  lighter / darker until they reach 4.6:1 on both gradient stops and on the
  glow drawn over them. `scripts/check-contrast.mjs` imports the same file
  and checks every slide × theme × accent (lowest: 4.61), plus the player's
  buttons.
- Text never overflows: every line is measured with a canvas in the same
  font, shrunk down to a minimum size and then cut with "…". Numbers from 1
  to 7 digits shrink to fit.
- Entrance: numbers count up (rAF, text only), bars grow and lines slide in
  with transform / opacity CSS animations; no layout work, no backdrop
  filters. With `prefers-reduced-motion` there is no count-up and no
  animation (the progress bar shows the current slide as full); slides still
  advance.
- Frame meter (⌥⌘F) while slides animate: 59–60 fps; the slowest single
  frame 18–45 ms, at the moment a new slide's SVG replaces the old one.

## PNG export

- "Save slide" / "Save all" build the slide's SVG again **without** the
  animation styles, at 1080 × 1920, load it into an `Image` (data URL), draw
  it on a canvas and encode a PNG. No new dependency, no html2canvas. Since
  the screen shows the same SVG, the export matches it by construction.
- The bytes go to the `save_recap_png` command as a raw request body (names
  in headers). Files: `~/Downloads/Wrapped/<period>/NN-<slide>.png`, e.g.
  `~/Downloads/Wrapped/week-2026-11-09/03-top_apps.png` (home from `HOME`,
  else `USERPROFILE`). The folder name must be lowercase letters, digits and
  `-`; the file `NN-<kind>.png`; `..`, separators and anything else are
  refused, the data must start with the PNG signature, and the folder must
  resolve (links followed) to a direct child of `~/Downloads/Wrapped`.
- A toast names the folder and has "Show in Finder" (`open -R`; Windows:
  `explorer /select,`), allowed only for paths inside that folder.
- Checked against the screen (light theme, green accent, and dark / pink) for
  intro, top_apps (a 70-character app id cut with "…"), git (7 digits) and
  summary: same font, colours, layout, nothing clipped.

## Notifications

- Settings → **Recaps**: Weekly / Monthly / Yearly switches, off by default
  (`recaps.notify.week|month|year`; `settings get|set` and `ctl set_settings`
  work).
- **When**: 09:00 local on the day after the period ends (weeks on Monday,
  months on the 1st); the year at 09:00 on 1 December (its in-progress
  recap). Only a period that has a recap is announced, once; the last one per
  kind is `internal.recap_notified.<kind>`.
- **Checks** run on the collector's existing minute tick: the first tick after
  start, then at most once an hour, and the first tick after a wake. No new
  thread or timer. With every switch off a check is a settings lookup.
- **After sleep** only the newest ready period is considered: one
  notification, never a burst of old ones.
- **First time on**: the kind's baseline is set to the period *before* the
  newest ready one, so at most that newest recap is announced, no older ones.
- **Permission** is asked (UNUserNotificationCenter, sound + alert) only when a
  switch is turned on; the answer is stored in `internal.notify_permission`
  and Settings shows "Notifications for Wrapped are turned off in macOS. Allow
  them in System Settings → Notifications → Wrapped" when it is denied.
- **Text**: one headline number, e.g. "Your week is ready: 32 h screen time,
  top app Xcode" (falls back to commits, photos, AI tokens, then active days).
- **Click**: the notification's identifier is the recap id; the delegate
  writes `open-recap` (the id) next to the database and opens the dashboard,
  which opens that recap (it looks for the flag at start and on focus, like
  `open-settings`). The dashboard also takes `--recap <id>`.
- `msg_send!` on the UserNotifications classes; the framework is linked in
  `platform/macos/notify.rs`. The center only exists inside an app bundle: a
  bare binary logs "not inside an app bundle" and posts nothing.
- Windows: compiles; no notifications until M9.
- Test: `wrapped-collector recap notify-test [week|month|year]` asks the
  running Wrapped.app to post the newest recap's notification now (marks
  nothing as announced).

## Measurements

- `recap_get` on the real database (read-only, release, median of 5): the
  longest period, year 2026 so far (323 days), **1.54 ms**; months 0.2–0.3 ms;
  `recap list` 0.62 ms. On a synthetic full year (80 k sessions, 29 k metric
  rows): year recap 18.1 ms, recap list 3.8 ms. No fix needed.
- Bundle (`ui/dist/assets`): JS 132.97 → 162.54 kB (gzip 45.93 → 56.86), CSS 29.91 → 34.59 kB (gzip 6.39 → 7.24).
- Binaries: collector 4,446,912 → 4,608,448 B; dashboard 14,273,264 →
  14,499,120 B (both built together with `custom-protocol`, as before).
- Collector idle physical footprint: 12 MB → 12 MB (peak 12 → 12), bare
  binaries side by side for 4 min on copies of the database, the new one with
  weekly and monthly notifications on. The installed (pre-M8) Wrapped.app
  showed 13 MB.

## Not verified

- Posting a notification, the permission prompt and the click were not run
  in an installed Wrapped.app (installing was out of scope here); the code
  path up to posting ran in a bare binary.
