//! SIGTERM / SIGINT (quit, with a final flush), SIGUSR1 (flush now) and
//! SIGUSR2 (look for requests from the dashboard / CLI, see `wrapped_core::ctl`).
//!
//! The signals are blocked in every thread and received synchronously by one
//! `sigwait` thread, which hands them to the main queue. Nothing runs in an
//! async signal handler, and the engine is only touched on the main thread.

use dispatch2::DispatchQueue;
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;

fn signal_set() -> libc::sigset_t {
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        for s in [libc::SIGTERM, libc::SIGINT, libc::SIGUSR1, libc::SIGUSR2] {
            libc::sigaddset(&mut set, s);
        }
        set
    }
}

/// Call before any other thread is started (threads inherit the mask).
pub fn install() {
    let set = signal_set();
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            loop {
                let mut sig: libc::c_int = 0;
                if unsafe { libc::sigwait(&set, &mut sig) } != 0 {
                    continue;
                }
                DispatchQueue::main().exec_async(move || on_signal(sig));
            }
        })
        .expect("spawn signal thread");
}

fn on_signal(sig: libc::c_int) {
    if sig == libc::SIGUSR1 {
        super::flush_now();
        return;
    }
    if sig == libc::SIGUSR2 {
        super::process_ctl();
        return;
    }
    super::log(if sig == libc::SIGTERM {
        "SIGTERM received, quitting"
    } else {
        "SIGINT received, quitting"
    });
    let mtm = MainThreadMarker::new().expect("main queue runs on the main thread");
    // Goes through applicationWillTerminate:, which flushes.
    NSApplication::sharedApplication(mtm).terminate(None);
}
