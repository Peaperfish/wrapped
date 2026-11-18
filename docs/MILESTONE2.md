# Milestone 2: scanned collectors (Git, Photos)

The two scanned collectors from DESIGN.md §7. Both are off by default.
Accepted differences from the design doc are in [DEVIATIONS.md](DEVIATIONS.md).

## What was built

- **Scan framework** (`engine.rs`, `collector.rs`). The engine starts each enabled
  scanned collector on the first minute tick at least 24 h after the start of
  its last completed scan (immediately if never scanned), never while paused,
  plus on demand (`wrapped-collector scan`). Each scan runs on its own
  background thread (macOS: background QoS; child processes get
  `PRIO_DARWIN_BG`), emits items through the `Sink`, and the shared batched
  writer persists them: early when 1,000 records are pending (the scan thread
  wakes the main thread through the platform's waker), and when the scan ends.
  At 5,000 pending records the scan thread waits for the writer, so memory
  stays bounded however much a scan finds. Sleep, power-off, quit, pause and
  turning the collector off cancel a scan; it saves its progress and the next
  tick resumes it. Turning off with delete, or deleting the data, aborts the
  scan and drops its output.
- **Incremental state** is written in the same transaction as the items
  emitted before it, so a cancel or crash never loses or skips data.
- **Git** (`collectors/git.rs`): walks `git.folders` (no symlinks; skips
  hidden dirs, `node_modules`, `target`, `build*`, `.venv`), finds work trees
  (`.git` dir or file) and bare repos (`*.git`), and per repo runs
  `git for-each-ref` (ref tips), then, only if the tips changed,
  `git log --stdin` with the new tips and `^` the tips seen last time. Stored
  per commit: an id, author time with the author's own offset, repo folder
  name. Only commits whose author email is in `git.author_emails`. The id
  hashes (author email, author time in seconds, author offset, first line of
  the message), so clones, rebases (`--force-rebase` too), amends that keep
  the message and cherry-picks count once; the message line only goes into
  the hash and is never stored or logged. Commit hashes are kept only as the
  ref tips in `scan_state` (the incremental watermark).
- **Photos** (`collectors/photos.rs`, `scan/exif.rs`): walks
  `photos.folders` (same rules, plus `*.photoslibrary`), files with an
  extension in `photos.extensions`. Unchanged files (same salted path hash,
  size, mtime) are not opened. Reads EXIF only, at most 2 MB per file.
  Stored per shot: capture time, make, model, lens, focal length, aperture,
  shutter, ISO. Never paths, names or pixels.
- **Settings**: `git.folders` (`["~/code"]`), `git.author_emails` (set from
  `git config --global user.email` when git is turned on), `photos.folders`
  (`["~/Pictures"]`), `photos.extensions` (jpg jpeg heic heif arw dng).
- **Metrics** (recomputed only for days that got new items): `git/commits`,
  `commits_by_repo`, `commits_by_hour`; `photos/shots`, `shots_by_camera`,
  `shots_by_lens`, `shots_by_hour`, `shots_by_focal_length` (whole mm).
- **Schema v2**: `items` and `scan_files` (see DEVIATIONS.md). Export includes
  `items`.
- **CLI**: `wrapped-collector scan [git|photos|all]` prints per-collector
  counters and the process's wall/CPU time, disk bytes read/written, peak
  physical footprint and database size. With a collector running on the same
  database it asks that collector to scan at its next tick instead.
  `wrapped-collector git emails` lists every author email in `git.folders`
  with its distinct commit count (same identity as scans) and repos, marking
  the ones in `git.author_emails`; read-only, prints to the terminal only, so
  you can pick old addresses to add with `settings set git.author_emails`.
- **Changing `git.author_emails`** (adding or removing an address) resets
  git's data and incremental state, so the next scan reads the full history
  again and counts exactly the listed addresses; setting the same list again
  changes nothing. `git.folders` needs no reset (a new repo has no watermark;
  a removed one simply stops being scanned), and no photos setting changes
  how a shot is identified or counted.

## Running it

```sh
export WRAPPED_DB=~/Documents/wrapped-notes/m2-scratch.db   # any test database
wrapped-collector settings set collector.git.enabled true     # also stores git.author_emails
wrapped-collector settings set collector.photos.enabled true
wrapped-collector settings set photos.folders '["~/Pictures", "/Volumes/Card/DCIM"]'
wrapped-collector scan git
wrapped-collector scan photos
wrapped-collector stats 2026-11-14
wrapped-collector git emails                                   # which emails to count
```

## Decisions

- **git CLI, not a Rust git crate.** `gix` adds several MB to the binary and
  a long compile; `git2` links libgit2 (C, plus OpenSSL/libssh2 unless
  trimmed) and needs a C toolchain on Windows. Anyone with repositories has
  `git` installed (Git for Windows ships `git.exe`). Per repository a scan
  runs `for-each-ref`, and `log` only when a ref moved; each process exits
  when done, nothing stays running. The commands are pinned to read-only
  behaviour: `GIT_OPTIONAL_LOCKS=0`, `core.fsmonitor=false` (no daemon),
  `log.showSignature=false` (no gpg), `--no-replace-objects`, revisions on
  stdin (no command-line length limit).
- **`kamadak-exif` for IFD parsing, our own container handling.** Checked:
  `kamadak-exif` 0.6 (one small dependency) parses JPEG and HEIF with seeks
  and bounded reads, but for TIFF-based files (ARW, DNG) `read_from_container`
  reads the **whole file**; `nom-exif` 3.8 handles all three but pulls in
  `nom`, `regex`, `chrono`, `bytes`, `tracing`, `thiserror`,
  `iso6709parse`, and grows its buffer up to 1 GB. So: JPEG segments are
  walked by seeking to the APP1 Exif segment; HEIF goes through
  `kamadak-exif`'s ISOBMFF reader on a capped reader; TIFF raws are read as a
  64 KB window, extended only as far as IFD0, the Exif IFD and the values of
  the kept tags reach (never past 2 MB), and parsed with `continue_on_error`. Every read goes through a
  counting reader that refuses to pass the 2 MB cap.
- **Shot id** = SipHash-2-4 ×2 (128 bits, fixed keys, implemented in
  `scan/mod.rs` so ids never change with a library version) of make, model,
  body serial, DateTimeOriginal, SubSecTimeOriginal, image number.
- **Path hashes are salted** (random per database, stored in the collector's
  `scan_state`), so the cache cannot be matched against guessed paths.
- **Background priority via platform hooks**: core has no OS code, so the
  platform layer passes `ScanHooks { thread_setup, command_setup }`.
- **Day of a commit** = local date in the author's own offset; **day of a
  photo** = OffsetTimeOriginal's local date, otherwise the camera time as
  written (stored with `time_off` NULL).

## Measurements (release build, scratch database, 2026-11-16)

`~/code`: 6 repositories (860 folders walked). `~/Pictures`: 2,551 photo files
(2,217 ARW, 228 JPG, 106 DNG). Numbers from `wrapped-collector scan` (kernel
counters via `proc_pid_rusage`, CPU via `getrusage`), one run each.

| Scan | Window | Wall | CPU (process + git) | Disk bytes read | Peak footprint | Result |
|---|---|---|---|---|---|---|
| git, first | | 66.9 s | 0.28 + 0.72 s | 110,592 | 3.5 MB | 15,522 commits read, 77 matched, 57 stored |
| git, incremental | | 19.9 s | 0.14 + 0.20 s | 0 | 3.2 MB | 6 repos unchanged |
| photos, first (before) | 256 KB | 15.4 s | 1.47 s | 619,667,456 | 5.6 MB | 2,486 parsed, 64 no date, 1 no EXIF → 2,349 shots |
| photos, first (after) | 64 KB | 4.0 s | 0.83 s | 159,907,840 | 5.4 MB | identical: 2,349 shots, 64 no date, 1 no EXIF, same top cameras / lenses |
| photos, incremental | | 2.5 s | 0.21 s | 0 | 3.4 MB | 2,551 unchanged |

- Git wall time is background throttling, not work: the same walk at normal
  priority takes 0.03 s; as a background process 4.5–22.7 s.
- After the window change a Sony ARW (Exif IFD ~5 KB in) costs exactly the
  64 KB first read; reads are now ~64 KB × raw files. The "before" run was the
  first ever read of those files, the "after" run came later in the same day,
  so cache state may differ; the bytes-read figure is what the scan asked for
  either way (the collector's own count: 613,854,854 → 157,633,961).
- Scratch database after both collectors: 840 KB.

## Known limitations

- On a Mac without the Xcode command line tools, `/usr/bin/git` is a stub that
  shows an install dialog. Turning git on resolves the author email with
  `git config`, which can trigger it.
- A git `log` that prints nothing for a long time (huge first scan of a big
  repo) only notices a cancel at its next output line.
- Commit counts include merges. Rewording a commit's first line (reword,
  `commit --amend -m`) makes it count again; two commits with the same
  author, second, offset and first line count once.
- A RAW and JPEG of the same shot count once only if the camera writes the
  same make, model, serial, time and subsec into both.
- Burst shots without sub-second time or image number in the same second
  count once.
- A cancel waits up to 2 s for the scan thread on the main thread (sleep,
  quit); after that it lets the thread finish on its own.
- Deleting a collector's data while it stays on re-scans everything on the
  next tick (its incremental state is deleted too).
