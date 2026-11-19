//! One hidden top-level window receives session, power, tray and timer messages.
//! Callbacks queue work so the engine is never borrowed through a reentrant WndProc.
use std::{
    cell::RefCell,
    collections::VecDeque,
    path::Path,
    ptr::{null, null_mut},
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Storage::Packaging::Appx::GetApplicationUserModelId,
        System::{
            Console::*, LibraryLoader::GetModuleHandleW, Power::*, RemoteDesktop::*,
            SystemInformation::GetTickCount64, SystemServices::GUID_SESSION_DISPLAY_STATUS,
            Threading::*,
        },
        UI::{Accessibility::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::w,
};
use wrapped_core::{Engine, Pause, SystemEvent, ctl};

enum Action {
    Event(SystemEvent),
    Tick,
    Menu,
    RestoreTray,
    Dashboard,
    Quit,
}
thread_local! {static QUEUE:RefCell<VecDeque<Action>>=const{RefCell::new(VecDeque::new())};}
thread_local! {
    // End-session is a synchronous Windows message. Flush before answering it.
    static SHUTDOWN_ENGINE: RefCell<std::rc::Weak<RefCell<Engine>>> = const { RefCell::new(std::rc::Weak::new()) };
}
fn queue(a: Action) {
    QUEUE.with_borrow_mut(|q| q.push_back(a));
}
fn event(e: SystemEvent) {
    queue(Action::Event(e));
}
fn process_id(hwnd: HWND) -> u32 {
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    pid
}
fn identity(pid: u32) -> Option<(String, String)> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 32768];
        let mut len = buf.len() as u32;
        let image = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) != 0;
        let name = if image {
            String::from_utf16_lossy(&buf[..len as usize])
                .rsplit('\\')
                .next()
                .unwrap_or("")
                .to_owned()
        } else {
            String::new()
        };
        len = buf.len() as u32;
        let id = if GetApplicationUserModelId(process, &mut len, buf.as_mut_ptr()) == 0 && len > 1 {
            String::from_utf16_lossy(&buf[..len as usize - 1])
        } else {
            name.to_lowercase()
        };
        CloseHandle(process);
        (!id.is_empty()).then_some((id, name))
    }
}
unsafe extern "system" fn hosted(child: HWND, param: LPARAM) -> i32 {
    let root = unsafe { GetAncestor(child, GA_ROOT) };
    if process_id(root) != process_id(child)
        && let Some(pair) = identity(process_id(child))
    {
        let found = unsafe { &mut *(param as *mut Option<(String, String)>) };
        // Frame hosts can contain helper windows as well as the packaged app.
        // Keep an executable fallback while looking for the package's AUMID.
        let packaged = pair.0.contains('!');
        if packaged || found.is_none() {
            *found = Some(pair);
        }
        if packaged {
            return 0;
        }
    }
    1
}
fn foreground(hwnd: HWND) {
    if let Some((mut bundle_id, mut name)) = identity(process_id(hwnd)) {
        if name.eq_ignore_ascii_case("ApplicationFrameHost.exe") {
            let mut app: Option<(String, String)> = None;
            unsafe {
                EnumChildWindows(hwnd, Some(hosted), (&mut app as *mut _) as isize);
            }
            if let Some(found) = app {
                (bundle_id, name) = found;
            }
        }
        event(SystemEvent::FrontApp { bundle_id, name });
    }
}
unsafe extern "system" fn switched(
    _: HWINEVENTHOOK,
    _: u32,
    hwnd: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    foreground(hwnd)
}
static WINDOW: std::sync::atomic::AtomicPtr<std::ffi::c_void> =
    std::sync::atomic::AtomicPtr::new(null_mut());
static TASKBAR_CREATED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
use std::sync::atomic::Ordering;
unsafe extern "system" fn interrupt(kind: u32) -> i32 {
    if !matches!(kind, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        return 0;
    }
    let hwnd = WINDOW.load(Ordering::Acquire);
    if hwnd.is_null() {
        return 0;
    }
    unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) }
}
fn locked_at_start() -> bool {
    unsafe {
        let mut ptr = null_mut();
        let mut bytes = 0;
        if WTSQuerySessionInformationW(
            null_mut(),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut ptr,
            &mut bytes,
        ) == 0
        {
            return false;
        }
        let locked = bytes as usize >= size_of::<WTSINFOEXW>() && {
            let info = &*ptr.cast::<WTSINFOEXW>();
            info.Level == 1
                && info.Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_LOCK as i32
        };
        WTSFreeMemory(ptr.cast());
        locked
    }
}
const TRAY: u32 = WM_APP + 1;
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        m if m != 0 && m == TASKBAR_CREATED.load(Ordering::Relaxed) => queue(Action::RestoreTray),
        WM_TIMER => queue(Action::Tick),
        WM_CLOSE => queue(Action::Quit),
        WM_QUERYENDSESSION => {
            SHUTDOWN_ENGINE.with_borrow(|weak| {
                if let Some(state) = weak.upgrade()
                    && let Ok(mut engine) = state.try_borrow_mut()
                    && let Err(e) = engine.flush()
                {
                    eprintln!("session ending: {e}");
                }
            });
            return 1;
        }
        WM_ENDSESSION if wp != 0 => {
            SHUTDOWN_ENGINE.with_borrow(|weak| {
                if let Some(state) = weak.upgrade()
                    && let Ok(mut engine) = state.try_borrow_mut()
                {
                    if let Err(e) = engine
                        .handle(SystemEvent::WillPowerOff)
                        .and_then(|_| engine.shutdown())
                    {
                        eprintln!("session shutdown: {e}");
                    }
                } else {
                    event(SystemEvent::WillPowerOff);
                }
            });
            queue(Action::Quit);
        }
        WM_WTSSESSION_CHANGE => match wp as u32 {
            WTS_SESSION_LOCK => event(SystemEvent::ScreenLocked),
            WTS_SESSION_UNLOCK => {
                event(SystemEvent::ScreenUnlocked);
                foreground(unsafe { GetForegroundWindow() });
            }
            _ => {}
        },
        WM_POWERBROADCAST => match wp as u32 {
            PBT_APMSUSPEND => event(SystemEvent::WillSleep),
            PBT_APMRESUMEAUTOMATIC => event(SystemEvent::DidWake),
            PBT_POWERSETTINGCHANGE if lp != 0 => {
                let setting = unsafe { &*(lp as *const POWERBROADCAST_SETTING) };
                if setting.DataLength == 4 {
                    event(if setting.Data[0] == 0 {
                        SystemEvent::ScreensSlept
                    } else {
                        SystemEvent::ScreensWoke
                    });
                }
            }
            _ => {}
        },
        TRAY => match lp as u32 {
            WM_RBUTTONUP => queue(Action::Menu),
            WM_LBUTTONUP => queue(Action::Dashboard),
            _ => {}
        },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    0
}
fn dashboard(db: &Path) -> Result<(), String> {
    if wrapped_core::windows_ipc::focus_dashboard(db) {
        return Ok(());
    }
    super::launch(db).map(|_| ())
}
fn menu(hwnd: HWND, paused: bool) -> u32 {
    unsafe {
        let m = CreatePopupMenu();
        if m.is_null() {
            return 0;
        }
        AppendMenuW(m, MF_STRING, 1, w!("Dashboard"));
        AppendMenuW(m, MF_STRING, 2, w!("Settings"));
        AppendMenuW(
            m,
            MF_STRING,
            3,
            if paused { w!("Resume") } else { w!("Pause") },
        );
        if !paused {
            AppendMenuW(m, MF_STRING, 5, w!("Pause for one hour"));
            AppendMenuW(m, MF_STRING, 6, w!("Pause until tomorrow"));
        }
        AppendMenuW(m, MF_STRING, 4, w!("Quit Wrapped"));
        let mut p = POINT::default();
        GetCursorPos(&mut p);
        SetForegroundWindow(hwnd);
        let selected = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            p.x,
            p.y,
            0,
            hwnd,
            null(),
        );
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        selected as u32
    }
}

pub fn run(
    db: &Path,
    state: &std::rc::Rc<RefCell<Engine>>,
    wake: &super::ipc::Event,
    setup: bool,
) -> std::io::Result<()> {
    unsafe {
        SHUTDOWN_ENGINE.set(std::rc::Rc::downgrade(state));
        let class = w!("WrappedCollector");
        let instance = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let hwnd = CreateWindowExW(
            0,
            class,
            w!("Wrapped Collector"),
            0,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if hwnd.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        WINDOW.store(hwnd, Ordering::Release);
        SetConsoleCtrlHandler(Some(interrupt), 1);
        TASKBAR_CREATED.store(
            RegisterWindowMessageW(w!("TaskbarCreated")),
            Ordering::Relaxed,
        );
        let sessions = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) != 0;
        if !sessions {
            eprintln!(
                "session notifications unavailable: {}",
                std::io::Error::last_os_error()
            );
        }
        let power = RegisterPowerSettingNotification(
            hwnd,
            &GUID_SESSION_DISPLAY_STATUS,
            DEVICE_NOTIFY_WINDOW_HANDLE,
        );
        let hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            null_mut(),
            Some(switched),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if hook.is_null() || SetTimer(hwnd, 1, 60000, None) == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Windows accepts a PNG icon resource directly (Vista and later).
        let icon = include_bytes!("../../../../../dashboard/icons/32x32.png");
        let own_icon = CreateIconFromResourceEx(
            icon.as_ptr(),
            icon.len() as u32,
            1,
            0x30000,
            0,
            0,
            LR_DEFAULTSIZE,
        );
        let mut tray = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: if own_icon.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                own_icon
            },
            ..Default::default()
        };
        for (dst, c) in tray.szTip.iter_mut().zip("Wrapped".encode_utf16()) {
            *dst = c;
        }
        Shell_NotifyIconW(NIM_ADD, &tray);
        if setup && let Err(e) = dashboard(db) {
            eprintln!("setup: {e}");
        }
        let mut locked = sessions && locked_at_start();
        if locked {
            event(SystemEvent::ScreenLocked)
        } else {
            foreground(GetForegroundWindow());
        }
        queue(Action::Tick);
        let mut quit = false;
        let apartment = ::windows::Win32::System::WinRT::RoInitialize(
            ::windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED,
        );
        let mut checked = None;
        let mut control = true;
        while !quit {
            {
                let mut borrowed = state.borrow_mut();
                let engine = &mut *borrowed;
                while let Some(a) = QUEUE.with_borrow_mut(|q| q.pop_front()) {
                    match a {
                        Action::RestoreTray => {
                            Shell_NotifyIconW(NIM_ADD, &tray);
                        }
                        Action::Event(e) => {
                            match e {
                                SystemEvent::ScreenLocked => locked = true,
                                SystemEvent::ScreenUnlocked => locked = false,
                                SystemEvent::FrontApp { .. } if locked => continue,
                                _ => {}
                            }
                            if matches!(e, SystemEvent::DidWake) {
                                checked = None;
                            }
                            if let Err(e) = engine.handle(e) {
                                eprintln!("{e}");
                            }
                        }
                        Action::Dashboard => {
                            if let Err(e) = dashboard(db) {
                                eprintln!("{e}");
                            }
                        }
                        Action::Menu => match menu(hwnd, engine.is_paused()) {
                            1 => queue(Action::Dashboard),
                            2 => {
                                let flag = ctl::open_settings_flag(db);
                                if std::fs::create_dir_all(ctl::dir_for(db))
                                    .and_then(|_| std::fs::write(flag, b"1"))
                                    .is_ok()
                                {
                                    queue(Action::Dashboard);
                                }
                            }
                            3 => {
                                let pause = if engine.is_paused() {
                                    Pause::Off
                                } else {
                                    Pause::Indefinite
                                };
                                if let Err(e) = engine.set_pause(pause) {
                                    eprintln!("{e}");
                                }
                            }
                            item @ (5 | 6) => {
                                let pause = if item == 5 {
                                    Pause::for_one_hour(engine.now())
                                } else {
                                    Pause::until_tomorrow(engine.now())
                                };
                                if let Err(e) = engine.set_pause(pause) {
                                    eprintln!("{e}");
                                }
                            }
                            4 => queue(Action::Quit),
                            _ => {}
                        },
                        Action::Tick => {
                            control = true;
                            if !engine.is_asleep()
                                && wrapped_core::recap::notify::any_enabled(engine.settings())
                                && checked.is_none_or(|t: std::time::Instant| {
                                    t.elapsed().as_secs() >= 3600
                                })
                            {
                                super::notify::check(db, engine);
                                checked = Some(std::time::Instant::now());
                            }
                            let mut info = LASTINPUTINFO {
                                cbSize: size_of::<LASTINPUTINFO>() as u32,
                                dwTime: 0,
                            };
                            if GetLastInputInfo(&mut info) != 0 {
                                event(SystemEvent::IdleTick {
                                    secs_since_input: (GetTickCount64() as u32)
                                        .wrapping_sub(info.dwTime)
                                        as f64
                                        / 1000.,
                                    display_assertions: vec![],
                                });
                            }
                        }
                        Action::Quit => quit = true,
                    }
                }
                if control {
                    for (id, req) in ctl::take_pending(db) {
                        let reply = match req.cmd.as_str() {
                            "recap_notify_test" => {
                                let reply = match super::notify::test(db, engine, &req.args) {
                                    Ok(v) => ctl::Reply::Ok(v),
                                    Err(e) => ctl::Reply::err("failed", e.to_string()),
                                };
                                ctl::reply(db, &id, &reply);
                                continue;
                            }
                            "shutdown" => {
                                quit = true;
                                Ok(())
                            }
                            "flush" => engine.flush().map(|_| ()).map_err(|e| e.to_string()),
                            "dashboard" => dashboard(db),
                            _ => {
                                let reply = engine.handle_request(&req);
                                if req.cmd == "set_settings"
                                    && req.args["settings"].as_object().is_some_and(|settings| {
                                        settings.keys().any(|k| k.starts_with("recaps.notify."))
                                    })
                                    && wrapped_core::recap::notify::any_enabled(engine.settings())
                                {
                                    checked = None;
                                    super::notify::permission(db, engine);
                                    let _ = wrapped_core::recap::notify::ensure_baselines(
                                        engine.store(),
                                        engine.settings(),
                                        engine.now(),
                                    );
                                }
                                if !matches!(req.cmd.as_str(), "ping" | "status")
                                    && let Err(e) = engine.flush()
                                {
                                    eprintln!("flush: {e}");
                                }
                                ctl::reply(db, &id, &reply);
                                continue;
                            }
                        };
                        ctl::reply(
                            db,
                            &id,
                            &match reply {
                                Ok(()) => ctl::Reply::Ok(serde_json::Value::Null),
                                Err(e) => ctl::Reply::err("failed", e),
                            },
                        );
                    }
                    if let Err(e) = engine.poll_scans() {
                        eprintln!("scan: {e}");
                    }
                    engine.take_scan_reports();
                    control = false;
                }
            }
            if quit {
                break;
            }
            let wait = MsgWaitForMultipleObjectsEx(
                1,
                &wake.handle(),
                u32::MAX,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
            control |= wait == WAIT_OBJECT_0;
            if wait == WAIT_FAILED {
                return Err(std::io::Error::last_os_error());
            }
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        if apartment.is_ok() {
            ::windows::Win32::System::WinRT::RoUninitialize();
        }
        Shell_NotifyIconW(NIM_DELETE, &tray);
        if !own_icon.is_null() {
            DestroyIcon(own_icon);
        }
        WINDOW.store(null_mut(), Ordering::Release);
        SetConsoleCtrlHandler(Some(interrupt), 0);
        UnhookWinEvent(hook);
        UnregisterPowerSettingNotification(power);
        WTSUnRegisterSessionNotification(hwnd);
        DestroyWindow(hwnd);
    }
    Ok(())
}
