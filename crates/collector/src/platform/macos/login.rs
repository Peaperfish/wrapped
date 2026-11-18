//! Launch at login via `SMAppService.mainApp` (macOS 13+).
//!
//! Registers the .app bundle the running executable is inside, so this only
//! works from the bundle built by `scripts/macos/bundle.sh` (a bare binary has
//! no bundle to register). macOS shows a one-time "background item added"
//! notice, and the item appears in System Settings → General → Login Items.

use objc2::rc::autoreleasepool;
use objc2_foundation::NSBundle;
use objc2_service_management::{SMAppService, SMAppServiceStatus};

fn status_text(s: SMAppServiceStatus) -> &'static str {
    match s {
        SMAppServiceStatus::NotRegistered => "not registered",
        SMAppServiceStatus::Enabled => "enabled",
        SMAppServiceStatus::RequiresApproval => {
            "requires approval in System Settings → General → Login Items"
        }
        SMAppServiceStatus::NotFound => "not registered (status: not found)",
        _ => "unknown",
    }
}

pub fn login(cmd: &str) -> i32 {
    autoreleasepool(|_| {
        let bundle = NSBundle::mainBundle().bundlePath().to_string();
        if !bundle.ends_with(".app") {
            eprintln!(
                "wrapped-collector: launch at login needs the .app bundle; this binary is not \
                 inside one ({bundle}). Build it with scripts/macos/bundle.sh and run \
                 Wrapped.app/Contents/MacOS/wrapped-collector login {cmd}"
            );
            return 1;
        }
        let service = unsafe { SMAppService::mainAppService() };
        let result = match cmd {
            "enable" => unsafe { service.registerAndReturnError() },
            "disable" => unsafe { service.unregisterAndReturnError() },
            _ => Ok(()),
        };
        if let Err(e) = result {
            eprintln!(
                "wrapped-collector: login {cmd} failed: {}",
                e.localizedDescription()
            );
            return 1;
        }
        let status = unsafe { service.status() };
        println!("launch at login for {bundle}: {}", status_text(status));
        0
    })
}
