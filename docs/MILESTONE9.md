# Milestone 9 · Windows port

The Windows x64 collector, tray, CLI and dashboard cross-build on macOS. Native Windows CI and interactive desktop verification are pending.

## Build and launch

Use Windows 10 or 11 x64, Rust stable with the MSVC host toolchain, Visual Studio Build Tools with Desktop development with C++ and the Windows SDK, Node.js 24 with npm, PowerShell, and the Microsoft Edge WebView2 Runtime. Git is needed for Git collection. From the checkout run

```powershell
.\scripts\windows\build.ps1
.\target\windows-package\Wrapped.exe
```

The build embeds the production frontend and prints the size of each packaged file. Copy the whole package to a stable folder writable by your account, for example `%LOCALAPPDATA%\Programs\Wrapped`. Launch `Wrapped.exe` there. A fresh database opens the setup checklist. Reopening the launcher brings up the dashboard. Use the tray menu to open Settings, pause for an hour, until tomorrow or indefinitely, resume, or quit.

From PowerShell in the installed folder

```powershell
.\wrapped-collector.exe login enable
.\wrapped-collector.exe login status
.\wrapped-collector.exe ctl status
.\wrapped-collector.exe now --count 2 --json
.\wrapped-collector.exe ctl shutdown
.\wrapped-collector.exe login disable
```

Launch at login uses the current user's `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Wrapped` value. Enable it again after moving the package. The CLI preserves terminal output and exit codes. The GUI launcher, dashboard helper calls and background Git processes run without opening a console.

## Database and controls

All three entry points default to `%LOCALAPPDATA%\Wrapped\wrapped.db`. Set `$env:WRAPPED_DB` to select another file. The dashboard also accepts `--db <path>`. Enabling login with an explicit override saves its absolute path in the startup command.

The collector holds an open file named by appending `.lock` to the database filename. Its Windows sharing mode refuses a second writer. After clearing stale control requests it publishes its PID and a new, session-local auto-reset event name. A client atomically publishes its request under `<database filename>.ctl` and signals the event. Windows releases the file and event handles when a process exits. The dashboard uses a separate `.dashboard.lock` and event to focus its existing window and check pending Settings and recap requests.

## Collection and hardware

Foreground changes arrive through an out-of-context WinEvent hook. Wrapped asks the owning process for its executable name and application user model ID. For ApplicationFrameHost it checks child-window owners. Identity depends on what the desktop and the target process expose. The collector runs below normal priority. Minute ticks read the session's last input time. The shared engine applies pause, exclusion and active-without-input settings. Session lock, unlock, display state, suspend, automatic resume and end-session messages come through a hidden top-level window. No elevated service, keyboard hook or injection is required. [Microsoft display notification reference](https://learn.microsoft.com/en-us/windows/win32/power/power-setting-guids)

| Reading | Windows source and coverage |
| --- | --- |
| CPU | English PDH Processor counters for logical processors reported by GetSystemInfo. Processor groups on machines with more than 64 logical CPUs need hardware validation. |
| Memory | GlobalMemoryStatusEx physical total and available. Used memory is their difference. |
| Disk | Cumulative PhysicalDisk total read and write bytes. The total can include removable disks. The Now sampler computes rates. Disk & Memory persists deltas with the existing sleep, pause and reset handling. |
| Network | IP Helper 64-bit octet counts per hardware interface, excluding loopback. |
| GPU | PDH GPU Engine utilization where the driver publishes it. Sum process contributions per engine and show the busiest engine. |
| Other sensors | Temperature, fan speed, compressed memory, memory pressure and swap readings remain absent. The port has no validated portable source for these measurements. |

Drive Health remains Milestone 10.

## Recaps

The same core scheduler determines which periods qualify. Opting in registers Wrapped's toast identity and the `wrapped-recap` protocol under the current user. A toast contains a database token and canonical recap ID. Activation resolves the locally registered database, writes its recap request and opens or focuses that dashboard. This also supports a click after the collector has exited. With a qualifying recap, `wrapped-collector.exe recap notify-test week` exercises a real submission without advancing the schedule.

## Verification

On the Mac, the frontend checks, production build and contrast checks passed. Rust formatting, native workspace Clippy and tests, and Windows GNU cross-target Clippy were run. The GNU release collector, dashboard and launcher linked. A local strip comparison on an intermediate GNU release reduced the dashboard from 28,837,279 to 18,097,664 bytes and the collector from 7,346,830 to 5,205,504 bytes. The Windows package build enables stripping. CI reports the native MSVC file sizes independently.

Windows CI builds the MSVC release, runs the Rust tests, and exercises the packaged collector with `scripts/windows/smoke.py`. The smoke test uses temporary app data, two same-stem databases including a Unicode override, competing writers, pause isolation, flush, clean shutdown, forced termination, restart, SQLite integrity checks and two live hardware samples. Each subprocess has a timeout or an owned cleanup path. Native regression tests check auto-reset events and verify the selected recap flag and dashboard wake without requiring a toast UI. A successful Windows job publishes the package as a workflow artifact.

Interactive verification still needs a Windows desktop:

- Open the tray and Settings, exercise each timed pause, restore a minimized dashboard, and restart Explorer. Enable login and sign out and in.
- Switch among Win32 and hosted UWP apps. Check exclusions, active-without-input timeout, lock and unlock, monitor power, suspend and resume, and shutdown while data is buffered.
- Deliver and click a real recap toast with the dashboard closed, already open, and after restarting the collector. Check notification permissions and Focus Assist.
- Compare CPU, disk, network and GPU readings with local counters on representative hardware, including after changing disks or interfaces.
- Measure steady-state memory, writes and CPU. Run paired gaming benchmarks with the collector enabled and disabled. Windows gaming impact and long-run overhead have not been measured.
