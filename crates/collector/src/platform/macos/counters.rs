//! System-wide disk and memory counters for the Disk & Memory collector.
//!
//! No permission, no entitlement, no child process:
//! * Disk bytes: IOKit `IOBlockStorageDriver` → `Statistics` → `Bytes (Read)` /
//!   `Bytes (Write)`, summed over drivers whose block storage device reports
//!   `Physical Interconnect Location = Internal`. That excludes disk images
//!   (location `File` / `Virtual`) and external drives; the built-in SD card
//!   reader (`Physical Interconnect = Secure Digital`) is excluded too, as
//!   removable media. APFS volumes and containers sit above the physical
//!   drivers, so nothing is counted twice.
//! * Swap in / out and compressed memory: `host_statistics64(HOST_VM_INFO64)`
//!   (`swapins`, `swapouts`, `compressor_page_count`) × the host page size,
//!   the numbers `vm_stat` prints.
//! * Memory pressure: `sysctl kern.memorystatus_vm_pressure_level`
//!   (1 normal, 2 warning, 4 critical).
//! * Boot time: `sysctl kern.boottime`.

use std::ffi::{CStr, c_char, c_void};

use objc2::msg_send;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;
use wrapped_core::probe::{DiskMemoryReading, PressureLevel, SystemProbe};

type IoObject = u32;
const IO_MAIN_PORT_DEFAULT: u32 = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    /// Consumes one reference to `matching`.
    fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: *mut c_void,
        existing: *mut IoObject,
    ) -> i32;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IOObjectRelease(object: IoObject) -> i32;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: *const c_void,
        allocator: *const c_void,
        options: u32,
    ) -> *mut c_void;
    fn IORegistryEntryGetParentEntry(
        entry: IoObject,
        plane: *const c_char,
        parent: *mut IoObject,
    ) -> i32;
}

// Mach calls declared here (libc marks its copies deprecated in favour of
// another crate; these are stable system ABI).
unsafe extern "C" {
    static mach_task_self_: libc::mach_port_t;
    fn mach_host_self() -> libc::mach_port_t;
    fn mach_port_deallocate(task: libc::mach_port_t, name: libc::mach_port_t) -> i32;
    fn host_page_size(host: libc::host_t, out: *mut libc::vm_size_t) -> i32;
}

/// Owns one IOKit object reference.
struct Io(IoObject);
impl Drop for Io {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { IOObjectRelease(self.0) };
        }
    }
}

/// A registry property as a retained Foundation object (CF types are
/// toll-free bridged), or `None`.
fn property(entry: IoObject, key: &str) -> Option<Retained<AnyObject>> {
    let key = NSString::from_str(key);
    let raw = unsafe {
        IORegistryEntryCreateCFProperty(
            entry,
            (&*key as *const NSString).cast(),
            std::ptr::null(),
            0,
        )
    };
    // +1 reference: owned by the Retained from here on.
    unsafe { Retained::from_raw(raw.cast::<AnyObject>()) }
}

fn dict_get(dict: &AnyObject, key: &str) -> Option<*mut AnyObject> {
    let key = NSString::from_str(key);
    let v: *mut AnyObject = unsafe { msg_send![dict, objectForKey: &*key] };
    (!v.is_null()).then_some(v)
}

fn dict_u64(dict: &AnyObject, key: &str) -> Option<u64> {
    let v = dict_get(dict, key)?;
    Some(unsafe { msg_send![v, unsignedLongLongValue] })
}

fn dict_string(dict: &AnyObject, key: &str) -> Option<String> {
    let v = dict_get(dict, key)?;
    // Only read it as a string if it is one (a registry value of another
    // type would crash when sent string messages).
    let obj: &AnyObject = unsafe { &*v };
    Some(obj.downcast_ref::<NSString>()?.to_string())
}

/// Whether a block storage driver sits on an internal, non-removable device.
fn is_internal(driver: IoObject) -> bool {
    let mut parent: IoObject = 0;
    let plane = c"IOService";
    if unsafe { IORegistryEntryGetParentEntry(driver, plane.as_ptr(), &mut parent) } != 0 {
        return false;
    }
    let parent = Io(parent);
    let Some(pc) = property(parent.0, "Protocol Characteristics") else {
        return false;
    };
    let location = dict_string(&pc, "Physical Interconnect Location");
    let interconnect = dict_string(&pc, "Physical Interconnect");
    location.as_deref() == Some("Internal") && interconnect.as_deref() != Some("Secure Digital")
}

/// (bytes read, bytes written) since boot, summed over internal drives.
fn disk_bytes() -> Option<(u64, u64)> {
    let class: &CStr = c"IOBlockStorageDriver";
    let matching = unsafe { IOServiceMatching(class.as_ptr()) };
    if matching.is_null() {
        return None;
    }
    let mut iter: IoObject = 0;
    if unsafe { IOServiceGetMatchingServices(IO_MAIN_PORT_DEFAULT, matching, &mut iter) } != 0 {
        return None;
    }
    let iter = Io(iter);
    let (mut read, mut written, mut found) = (0u64, 0u64, false);
    loop {
        let entry = Io(unsafe { IOIteratorNext(iter.0) });
        if entry.0 == 0 {
            break;
        }
        if !is_internal(entry.0) {
            continue;
        }
        let Some(stats) = property(entry.0, "Statistics") else {
            continue;
        };
        read = read.saturating_add(dict_u64(&stats, "Bytes (Read)").unwrap_or(0));
        written = written.saturating_add(dict_u64(&stats, "Bytes (Write)").unwrap_or(0));
        found = true;
    }
    found.then_some((read, written))
}

/// (swap-in bytes, swap-out bytes, compressed bytes).
fn vm_bytes() -> Option<(u64, u64, u64)> {
    unsafe {
        let host = mach_host_self();
        let mut page: libc::vm_size_t = 0;
        let mut info: libc::vm_statistics64 = std::mem::zeroed();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let rc_page = host_page_size(host, &mut page);
        let rc = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            (&mut info as *mut libc::vm_statistics64).cast(),
            &mut count,
        );
        mach_port_deallocate(mach_task_self_, host);
        if rc != 0 || rc_page != 0 || page == 0 {
            return None;
        }
        let page = page as u64;
        let swapins = info.swapins;
        let swapouts = info.swapouts;
        let compressor = u64::from(info.compressor_page_count);
        Some((swapins * page, swapouts * page, compressor * page))
    }
}

fn pressure() -> Option<PressureLevel> {
    let mut level: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    let name = c"kern.memorystatus_vm_pressure_level";
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&mut level as *mut libc::c_int).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    Some(match level {
        4 => PressureLevel::Critical,
        2 => PressureLevel::Warning,
        _ => PressureLevel::Normal,
    })
}

fn boot_time_ms() -> Option<i64> {
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::timeval>();
    let mut mib = [libc::CTL_KERN, libc::KERN_BOOTTIME];
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&mut tv as *mut libc::timeval).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then(|| tv.tv_sec * 1000 + i64::from(tv.tv_usec) / 1000)
}

/// One reading of every counter, or `None` if any part failed.
pub fn disk_memory() -> Option<DiskMemoryReading> {
    autoreleasepool(|_| {
        let (disk_bytes_read, disk_bytes_written) = disk_bytes()?;
        let (swap_in_bytes, swap_out_bytes, compressed_bytes) = vm_bytes()?;
        Some(DiskMemoryReading {
            disk_only: false,
            boot_time_ms: boot_time_ms()?,
            disk_bytes_read,
            disk_bytes_written,
            swap_in_bytes,
            swap_out_bytes,
            pressure: pressure()?,
            compressed_bytes,
        })
    })
}

/// The probe handed to the engine.
pub struct MacProbe;

impl SystemProbe for MacProbe {
    fn disk_memory(&self) -> Option<DiskMemoryReading> {
        disk_memory()
    }
}
