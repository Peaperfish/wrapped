//! macOS sources for the Now tab. Public APIs only, declared by hand (no
//! extra crates, no entitlement, no child process):
//!
//! * CPU per core: `host_processor_info(PROCESSOR_CPU_LOAD_INFO)`. The P / E
//!   cluster of each core: IORegistry `IODeviceTree:/cpus/cpuN` →
//!   `cluster-type` ("P" / "E"), read once.
//! * GPU: IOKit `IOAccelerator` → `PerformanceStatistics` →
//!   `Device Utilization %`.
//! * Memory: `host_statistics64(HOST_VM_INFO64)` (used = app + wired +
//!   compressed, Activity Monitor's definition), `hw.memsize`,
//!   `kern.memorystatus_vm_pressure_level`; swap size `vm.swapusage`; swap
//!   in / out counters from the same VM statistics.
//! * Disk: IOKit `IOBlockStorageDriver` statistics of internal drives (as the
//!   Disk & Memory collector does).
//! * Network: `sysctl NET_RT_IFLIST2` (64-bit byte counters per interface).
//! * Temperatures and fans: the AppleSMC IOKit user client (`T…` and `F…`
//!   keys). Key names differ per chip; only keys that have a confident label
//!   are reported.

use std::collections::BTreeMap;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::time::Instant;

use super::{CoreKind, Fan, NowSource, RawCore, RawIface, RawMem, RawNow, Temp};

type MachPort = u32;
type Kern = i32;
type IoObject = u32;

// ---------- FFI ----------

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> *mut c_void;
    fn IOServiceGetMatchingServices(main: u32, matching: *mut c_void, it: *mut IoObject) -> Kern;
    fn IOServiceGetMatchingService(main: u32, matching: *mut c_void) -> IoObject;
    fn IOIteratorNext(it: IoObject) -> IoObject;
    fn IOObjectRelease(o: IoObject) -> Kern;
    fn IORegistryEntryCreateCFProperty(
        entry: IoObject,
        key: *const c_void,
        alloc: *const c_void,
        options: u32,
    ) -> *mut c_void;
    fn IORegistryEntryGetParentEntry(
        entry: IoObject,
        plane: *const c_char,
        p: *mut IoObject,
    ) -> Kern;
    fn IORegistryEntryFromPath(main: u32, path: *const c_char) -> IoObject;
    fn IOServiceOpen(service: IoObject, owner: MachPort, kind: u32, conn: *mut IoObject) -> Kern;
    fn IOServiceClose(conn: IoObject) -> Kern;
    fn IOConnectCallStructMethod(
        conn: IoObject,
        selector: u32,
        input: *const c_void,
        input_size: usize,
        output: *mut c_void,
        output_size: *mut usize,
    ) -> Kern;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(cf: *const c_void);
    fn CFGetTypeID(cf: *const c_void) -> usize;
    fn CFNumberGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFNumberGetValue(n: *const c_void, kind: c_int, out: *mut c_void) -> u8;
    fn CFStringCreateWithCString(alloc: *const c_void, s: *const c_char, enc: u32) -> *mut c_void;
    fn CFStringGetCString(s: *const c_void, buf: *mut c_char, len: isize, enc: u32) -> u8;
    fn CFDictionaryGetValue(d: *const c_void, key: *const c_void) -> *const c_void;
    fn CFDataGetLength(d: *const c_void) -> isize;
    fn CFDataGetBytePtr(d: *const c_void) -> *const u8;
}

unsafe extern "C" {
    static mach_task_self_: MachPort;
    fn mach_host_self() -> MachPort;
    fn mach_port_deallocate(task: MachPort, name: MachPort) -> Kern;
    fn host_page_size(host: MachPort, out: *mut usize) -> Kern;
    fn host_statistics64(host: MachPort, flavor: c_int, out: *mut c_void, count: *mut u32) -> Kern;
    fn host_processor_info(
        host: MachPort,
        flavor: c_int,
        count: *mut u32,
        info: *mut *mut i32,
        info_count: *mut u32,
    ) -> Kern;
    fn vm_deallocate(task: MachPort, addr: usize, size: usize) -> Kern;
    fn sysctlbyname(
        name: *const c_char,
        old: *mut c_void,
        oldlen: *mut usize,
        new: *mut c_void,
        newlen: usize,
    ) -> c_int;
    fn sysctl(
        name: *mut c_int,
        namelen: u32,
        old: *mut c_void,
        oldlen: *mut usize,
        new: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

const CF_UTF8: u32 = 0x0800_0100;
const CF_SINT64: c_int = 4;
const CF_FLOAT64: c_int = 6;
const HOST_VM_INFO64: c_int = 4;
const PROCESSOR_CPU_LOAD_INFO: c_int = 2;

/// `vm_statistics64` (`<mach/vm_statistics.h>`, packed to 8).
#[repr(C)]
#[derive(Default)]
struct VmStat64 {
    free_count: u32,
    active_count: u32,
    inactive_count: u32,
    wire_count: u32,
    zero_fill_count: u64,
    reactivations: u64,
    pageins: u64,
    pageouts: u64,
    faults: u64,
    cow_faults: u64,
    lookups: u64,
    hits: u64,
    purges: u64,
    purgeable_count: u32,
    speculative_count: u32,
    decompressions: u64,
    compressions: u64,
    swapins: u64,
    swapouts: u64,
    compressor_page_count: u32,
    throttled_count: u32,
    external_page_count: u32,
    internal_page_count: u32,
    total_uncompressed_pages_in_compressor: u64,
}
const VM_STAT64_COUNT: u32 = (std::mem::size_of::<VmStat64>() / 4) as u32;

// ---------- CoreFoundation helpers ----------

/// An owned CF object.
struct Cf(*mut c_void);
impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

impl Cf {
    fn string(s: &str) -> Option<Cf> {
        let c = CString::new(s).ok()?;
        let p = unsafe { CFStringCreateWithCString(std::ptr::null(), c.as_ptr(), CF_UTF8) };
        (!p.is_null()).then_some(Cf(p))
    }
    fn is(&self, ty: unsafe extern "C" fn() -> usize) -> bool {
        unsafe { CFGetTypeID(self.0) == ty() }
    }
    fn get(&self, key: &str) -> Option<Borrowed> {
        if !self.is(CFDictionaryGetTypeID) {
            return None;
        }
        let k = Cf::string(key)?;
        let v = unsafe { CFDictionaryGetValue(self.0, k.0) };
        (!v.is_null()).then_some(Borrowed(v))
    }
    fn as_borrowed(&self) -> Borrowed {
        Borrowed(self.0)
    }
}

/// A CF object owned by a container (never released here).
#[derive(Clone, Copy)]
struct Borrowed(*const c_void);
impl Borrowed {
    fn f64(self) -> Option<f64> {
        if unsafe { CFGetTypeID(self.0) != CFNumberGetTypeID() } {
            return None;
        }
        let mut out = 0f64;
        let ok = unsafe { CFNumberGetValue(self.0, CF_FLOAT64, (&mut out as *mut f64).cast()) };
        (ok != 0).then_some(out)
    }
    fn u64(self) -> Option<u64> {
        if unsafe { CFGetTypeID(self.0) != CFNumberGetTypeID() } {
            return None;
        }
        let mut out = 0i64;
        let ok = unsafe { CFNumberGetValue(self.0, CF_SINT64, (&mut out as *mut i64).cast()) };
        (ok != 0).then(|| out.max(0) as u64)
    }
    fn string(self) -> Option<String> {
        if unsafe { CFGetTypeID(self.0) != CFStringGetTypeID() } {
            return None;
        }
        let mut buf = [0 as c_char; 128];
        let ok = unsafe { CFStringGetCString(self.0, buf.as_mut_ptr(), 128, CF_UTF8) };
        (ok != 0).then(|| {
            unsafe { CStr::from_ptr(buf.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
    }
    fn data(self) -> Option<Vec<u8>> {
        if unsafe { CFGetTypeID(self.0) != CFDataGetTypeID() } {
            return None;
        }
        let len = unsafe { CFDataGetLength(self.0) }.max(0) as usize;
        let p = unsafe { CFDataGetBytePtr(self.0) };
        (!p.is_null()).then(|| unsafe { std::slice::from_raw_parts(p, len) }.to_vec())
    }
}

/// An owned IOKit object reference.
struct Io(IoObject);
impl Drop for Io {
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe { IOObjectRelease(self.0) };
        }
    }
}

impl Io {
    fn property(&self, key: &str) -> Option<Cf> {
        let k = Cf::string(key)?;
        let p = unsafe { IORegistryEntryCreateCFProperty(self.0, k.0, std::ptr::null(), 0) };
        (!p.is_null()).then_some(Cf(p))
    }
}

/// Every registry entry of a class.
fn services(class: &CStr) -> Vec<Io> {
    let matching = unsafe { IOServiceMatching(class.as_ptr()) };
    if matching.is_null() {
        return Vec::new();
    }
    let mut it: IoObject = 0;
    if unsafe { IOServiceGetMatchingServices(0, matching, &mut it) } != 0 {
        return Vec::new();
    }
    let it = Io(it);
    let mut out = Vec::new();
    loop {
        let e = Io(unsafe { IOIteratorNext(it.0) });
        if e.0 == 0 {
            break;
        }
        out.push(e);
    }
    out
}

// ---------- sysctl ----------

fn sysctl_by_name<T: Default>(name: &CStr) -> Option<T> {
    let mut v = T::default();
    let mut len = std::mem::size_of::<T>();
    let rc = unsafe {
        sysctlbyname(
            name.as_ptr(),
            (&mut v as *mut T).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len == std::mem::size_of::<T>()).then_some(v)
}

fn sysctl_string(name: &CStr) -> Option<String> {
    let mut len = 0usize;
    if unsafe {
        sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || len == 0
    {
        return None;
    }
    let mut buf = vec![0u8; len];
    if unsafe {
        sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buf.truncate(len);
    while buf.last() == Some(&0) {
        buf.pop();
    }
    String::from_utf8(buf).ok()
}

// ---------- readers ----------

/// The host port, deallocated on drop.
struct HostPort(MachPort);
impl HostPort {
    fn new() -> Self {
        Self(unsafe { mach_host_self() })
    }
}
impl Drop for HostPort {
    fn drop(&mut self) {
        unsafe { mach_port_deallocate(mach_task_self_, self.0) };
    }
}

/// Cluster of logical CPU `i` from the device tree.
fn core_kinds() -> Vec<Option<CoreKind>> {
    let n = sysctl_by_name::<i32>(c"hw.logicalcpu").unwrap_or(0).max(0);
    (0..n)
        .map(|i| {
            let path = CString::new(format!("IODeviceTree:/cpus/cpu{i}")).ok()?;
            let e = Io(unsafe { IORegistryEntryFromPath(0, path.as_ptr()) });
            if e.0 == 0 {
                return None;
            }
            let data = e.property("cluster-type")?.as_borrowed().data()?;
            match data.first() {
                Some(b'P') => Some(CoreKind::Performance),
                Some(b'E') => Some(CoreKind::Efficiency),
                _ => None,
            }
        })
        .collect()
}

fn read_cores(kinds: &[Option<CoreKind>]) -> Option<Vec<RawCore>> {
    let host = HostPort::new();
    let mut count = 0u32;
    let mut info: *mut i32 = std::ptr::null_mut();
    let mut info_count = 0u32;
    let rc = unsafe {
        host_processor_info(
            host.0,
            PROCESSOR_CPU_LOAD_INFO,
            &mut count,
            &mut info,
            &mut info_count,
        )
    };
    if rc != 0 || info.is_null() {
        return None;
    }
    let cores = (0..count as usize)
        .filter(|i| (i + 1) * 4 <= info_count as usize)
        .map(|i| {
            // user, system, idle, nice (u32 tick counters stored as i32).
            let t = |k: usize| unsafe { *info.add(i * 4 + k) } as u32;
            RawCore {
                busy: t(0).wrapping_add(t(1)).wrapping_add(t(3)),
                idle: t(2),
                kind: kinds.get(i).copied().flatten(),
            }
        })
        .collect::<Vec<_>>();
    unsafe { vm_deallocate(mach_task_self_, info as usize, info_count as usize * 4) };
    (!cores.is_empty()).then_some(cores)
}

fn read_gpu() -> Option<f64> {
    services(c"IOAccelerator")
        .iter()
        .filter_map(|e| {
            e.property("PerformanceStatistics")?
                .get("Device Utilization %")?
                .f64()
        })
        .reduce(f64::max)
}

struct VmInfo {
    mem: RawMem,
    swap_in: u64,
    swap_out: u64,
}

fn read_vm(total: u64) -> Option<VmInfo> {
    let host = HostPort::new();
    let mut page = 0usize;
    let mut st = VmStat64::default();
    let mut count = VM_STAT64_COUNT;
    let ok = unsafe {
        host_page_size(host.0, &mut page) == 0
            && host_statistics64(
                host.0,
                HOST_VM_INFO64,
                (&mut st as *mut VmStat64).cast(),
                &mut count,
            ) == 0
    };
    if !ok || page == 0 {
        return None;
    }
    let page = page as u64;
    let app = u64::from(st.internal_page_count).saturating_sub(u64::from(st.purgeable_count));
    let used = (app + u64::from(st.wire_count) + u64::from(st.compressor_page_count)) * page;
    let pressure = sysctl_by_name::<i32>(c"kern.memorystatus_vm_pressure_level").unwrap_or(1);
    Some(VmInfo {
        mem: RawMem {
            used: used.min(total),
            total,
            compressed: u64::from(st.compressor_page_count) * page,
            pressure: pressure.clamp(1, 4) as u8,
        },
        swap_in: st.swapins * page,
        swap_out: st.swapouts * page,
    })
}

/// `struct xsw_usage` (`vm.swapusage`).
#[repr(C)]
#[derive(Default)]
struct SwapUsage {
    total: u64,
    avail: u64,
    used: u64,
    page_size: u32,
    encrypted: u8,
}

fn read_swap() -> Option<(u64, u64)> {
    let mut v = SwapUsage::default();
    let mut len = std::mem::size_of::<SwapUsage>();
    let rc = unsafe {
        sysctlbyname(
            c"vm.swapusage".as_ptr(),
            (&mut v as *mut SwapUsage).cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some((v.used, v.total))
}

fn is_internal(driver: &Io) -> bool {
    let mut parent: IoObject = 0;
    if unsafe { IORegistryEntryGetParentEntry(driver.0, c"IOService".as_ptr(), &mut parent) } != 0 {
        return false;
    }
    let parent = Io(parent);
    let Some(pc) = parent.property("Protocol Characteristics") else {
        return false;
    };
    let s = |k: &str| pc.get(k).and_then(Borrowed::string);
    s("Physical Interconnect Location").as_deref() == Some("Internal")
        && s("Physical Interconnect").as_deref() != Some("Secure Digital")
}

fn read_disk() -> Option<(u64, u64)> {
    let (mut r, mut w, mut found) = (0u64, 0u64, false);
    for d in services(c"IOBlockStorageDriver") {
        if !is_internal(&d) {
            continue;
        }
        let Some(stats) = d.property("Statistics") else {
            continue;
        };
        let g = |k: &str| stats.get(k).and_then(Borrowed::u64).unwrap_or(0);
        r = r.saturating_add(g("Bytes (Read)"));
        w = w.saturating_add(g("Bytes (Write)"));
        found = true;
    }
    found.then_some((r, w))
}

fn rd_u16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_ne_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn rd_u32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn rd_u64(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// 64-bit (bytes in, bytes out) of interface `index` from the interface MIB
/// (`struct ifmibdata`, packed to 4: ibytes at 116, obytes at 124). The
/// routing-socket list carries these counters too, but truncated to 32 bits
/// and rounded to 4 KB, so it is used only to find the interfaces.
fn iface_bytes(index: u32) -> Option<(u64, u64)> {
    const CTL_NET: c_int = 4;
    const PF_LINK: c_int = 18;
    const NETLINK_GENERIC: c_int = 0;
    const IFMIB_IFDATA: c_int = 2;
    const IFDATA_GENERAL: c_int = 1;
    let mut mib = [
        CTL_NET,
        PF_LINK,
        NETLINK_GENERIC,
        IFMIB_IFDATA,
        index as c_int,
        IFDATA_GENERAL,
    ];
    let mut buf = [0u8; 512];
    let mut len = buf.len();
    let rc = unsafe {
        sysctl(
            mib.as_mut_ptr(),
            6,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len < 132 {
        return None;
    }
    Some((rd_u64(&buf, 116)?, rd_u64(&buf, 124)?))
}

/// Byte counters of every non-loopback interface that is up. The interface
/// list comes from the routing socket (`if_msghdr2`).
fn read_net() -> Option<Vec<RawIface>> {
    const CTL_NET: c_int = 4;
    const PF_ROUTE: c_int = 17;
    const NET_RT_IFLIST2: c_int = 6;
    const RTM_IFINFO2: u8 = 0x12;
    const IFF_UP: i32 = 0x1;
    const IFF_LOOPBACK: i32 = 0x8;
    // `struct if_msghdr2`: flags at 8, index at 12, `ifm_data` at 32 (its
    // first byte is the interface type).
    const DATA: usize = 32;

    let mut mib = [CTL_NET, PF_ROUTE, 0, 0, NET_RT_IFLIST2, 0];
    let mut len = 0usize;
    if unsafe {
        sysctl(
            mib.as_mut_ptr(),
            6,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    // The list can grow between the two calls.
    let mut buf = vec![0u8; len + len / 8 + 512];
    let mut len = buf.len();
    if unsafe {
        sysctl(
            mib.as_mut_ptr(),
            6,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    let buf = &buf[..len];
    let mut out = Vec::new();
    let mut off = 0;
    while off + 4 <= buf.len() {
        let msglen = rd_u16(buf, off)? as usize;
        if msglen == 0 || off + msglen > buf.len() {
            break;
        }
        let msg = &buf[off..off + msglen];
        if msg[3] == RTM_IFINFO2 && msglen > DATA {
            let flags = rd_u32(msg, 8)? as i32;
            let index = u32::from(rd_u16(msg, 12)?);
            let itype = msg[DATA];
            if flags & IFF_UP != 0
                && flags & IFF_LOOPBACK == 0
                && itype != 24
                && let Some((rx, tx)) = iface_bytes(index)
            {
                out.push(RawIface { id: index, rx, tx });
            }
        }
        off += msglen;
    }
    Some(out)
}

// ---------- AppleSMC ----------

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcVers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu: u32,
    gpu: u32,
    mem: u32,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcKeyInfo {
    size: u32,
    kind: u32,
    attrs: u8,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SmcData {
    key: u32,
    vers: SmcVers,
    plimit: SmcPLimit,
    info: SmcKeyInfo,
    result: u8,
    status: u8,
    cmd: u8,
    data32: u32,
    bytes: [u8; 32],
}
const _: () = assert!(std::mem::size_of::<SmcData>() == 80);

/// Sensors read per group (CPU performance cores alone have dozens of keys).
const MAX_SENSORS_PER_GROUP: usize = 4;

const SMC_READ_BYTES: u8 = 5;
const SMC_READ_INDEX: u8 = 8;
const SMC_READ_INFO: u8 = 9;
const SMC_HANDLE_YPC: u32 = 2;

fn fourcc(s: &str) -> u32 {
    s.bytes().take(4).fold(0, |a, b| (a << 8) | u32::from(b))
}
fn fourcc_str(v: u32) -> String {
    v.to_be_bytes().iter().map(|&b| b as char).collect()
}

struct Smc {
    conn: IoObject,
    /// Temperature keys to read each time: (key, size, group label).
    temps: Vec<(u32, u32, String)>,
    fans: Vec<FanKeys>,
}

struct FanKeys {
    name: String,
    actual: u32,
    /// Fixed by the hardware: read once.
    min: Option<f64>,
    max: Option<f64>,
}

impl Drop for Smc {
    fn drop(&mut self) {
        unsafe { IOServiceClose(self.conn) };
    }
}

const CPU_P: &str = "CPU performance cores";

/// A sensor group by key. Only keys known to mean this are listed; every
/// other `T…` key is left out rather than guessed.
///
/// Measured on a Mac14,9 (M2 Pro) with 8 busy `yes` processes for 20 s:
/// * `Tp1[A-z]` and `Tp2[A-z]` rose 55-65 C (idle readings 0-8 C while the cluster
///   is power-gated, 58-74 C under load): the P cores. The other `Tp0?` /
///   `Tp16-18` keys rose only 4-16 C from 44-66 C, so they are not cores.
/// * `Te0?`: the E cores. `Tg???`: GPU (the GPU was not loaded, they rose
///   with the shared die).
/// * `TH`: storage, `TB`: battery, `TaLP` / `TaRF`: airflow left / right,
///   `TW`: Wi-Fi.
///
/// Other chips may number their sensors differently; there a group may be
/// missing rather than wrong.
fn temp_group(key: &str) -> Option<&'static str> {
    if matches!(key, "TaLP" | "TaRF") {
        return Some("Airflow");
    }
    let b = key.as_bytes();
    Some(match key.get(..2)? {
        "Tp" if matches!(b.get(2), Some(b'1' | b'2'))
            && b.get(3).is_some_and(u8::is_ascii_alphabetic) =>
        {
            CPU_P
        }
        "Te" => "CPU efficiency cores",
        "Tg" => "GPU",
        "TH" | "TN" => "SSD",
        "TB" => "Battery",
        "TW" => "Wi-Fi",
        _ => return None,
    })
}

impl Smc {
    fn open() -> Option<Smc> {
        let matching = unsafe { IOServiceMatching(c"AppleSMC".as_ptr()) };
        if matching.is_null() {
            return None;
        }
        let svc = Io(unsafe { IOServiceGetMatchingService(0, matching) });
        if svc.0 == 0 {
            return None;
        }
        let mut conn: IoObject = 0;
        if unsafe { IOServiceOpen(svc.0, mach_task_self_, 0, &mut conn) } != 0 {
            return None;
        }
        let mut smc = Smc {
            conn,
            temps: Vec::new(),
            fans: Vec::new(),
        };
        smc.discover();
        Some(smc)
    }

    fn call(&self, input: &SmcData) -> Option<SmcData> {
        let mut out = SmcData::default();
        let mut size = std::mem::size_of::<SmcData>();
        let rc = unsafe {
            IOConnectCallStructMethod(
                self.conn,
                SMC_HANDLE_YPC,
                (input as *const SmcData).cast(),
                std::mem::size_of::<SmcData>(),
                (&mut out as *mut SmcData).cast(),
                &mut size,
            )
        };
        (rc == 0 && out.result == 0).then_some(out)
    }

    fn info(&self, key: u32) -> Option<SmcKeyInfo> {
        let input = SmcData {
            key,
            cmd: SMC_READ_INFO,
            ..SmcData::default()
        };
        let out = self.call(&input)?;
        (out.info.size > 0 && out.info.size <= 32).then_some(out.info)
    }

    fn bytes(&self, key: u32, size: u32) -> Option<[u8; 32]> {
        let mut input = SmcData {
            key,
            cmd: SMC_READ_BYTES,
            ..SmcData::default()
        };
        input.info.size = size;
        Some(self.call(&input)?.bytes)
    }

    /// `flt ` value (little-endian f32 on Apple Silicon).
    fn float(&self, key: u32, size: u32) -> Option<f64> {
        let b = self.bytes(key, size)?;
        let v = f64::from(f32::from_le_bytes(b[..4].try_into().ok()?));
        v.is_finite().then_some(v)
    }

    fn key_at(&self, index: u32) -> Option<u32> {
        let input = SmcData {
            cmd: SMC_READ_INDEX,
            data32: index,
            ..SmcData::default()
        };
        Some(self.call(&input)?.key)
    }

    /// Lists the keys once; later reads touch only the useful ones.
    fn discover(&mut self) {
        let count_key = fourcc("#KEY");
        let Some(info) = self.info(count_key) else {
            return;
        };
        let Some(b) = self.bytes(count_key, info.size) else {
            return;
        };
        let count = u32::from_be_bytes([b[0], b[1], b[2], b[3]]).min(4000);
        let flt = fourcc("flt ");
        let mut fan_count = 0usize;
        for i in 0..count {
            let Some(key) = self.key_at(i) else { continue };
            let name = fourcc_str(key);
            if name == "FNum" {
                fan_count = self.bytes(key, 1).map_or(0, |b| usize::from(b[0]));
                continue;
            }
            let Some(group) = temp_group(&name).filter(|_| name.starts_with('T')) else {
                continue;
            };
            let Some(info) = self.info(key) else { continue };
            if info.kind != flt || info.size != 4 {
                continue;
            }
            // Inactive or absent sensors read 0 or nonsense: keep only keys
            // that currently give a plausible temperature. The P-core keys
            // read ~0 while the cluster sleeps, so they are kept regardless.
            if group == CPU_P || self.float(key, 4).is_some_and(|v| plausible_temp(group, v)) {
                self.temps.push((key, 4, group.to_string()));
            }
        }
        self.thin_temps();
        for n in 0..fan_count.min(8) {
            let k = |s: &str| {
                let key = fourcc(&format!("F{n}{s}"));
                self.info(key)
                    .filter(|i| i.kind == flt && i.size == 4)
                    .map(|_| key)
            };
            if let Some(actual) = k("Ac") {
                self.fans.push(FanKeys {
                    name: format!("Fan {}", n + 1),
                    actual,
                    min: k("Mn").and_then(|k| self.float(k, 4)),
                    max: k("Mx").and_then(|k| self.float(k, 4)),
                });
            }
        }
    }

    /// An SMC read costs about 0.2 ms, so each group keeps a few evenly
    /// spaced sensors (the group is reported as their mean).
    fn thin_temps(&mut self) {
        let mut by_group: BTreeMap<String, Vec<(u32, u32, String)>> = BTreeMap::new();
        for t in self.temps.drain(..) {
            by_group.entry(t.2.clone()).or_default().push(t);
        }
        for (_, keys) in by_group {
            let n = keys.len();
            let take = n.min(MAX_SENSORS_PER_GROUP);
            for i in 0..take {
                self.temps.push(keys[i * n / take].clone());
            }
        }
    }

    fn read_temps(&self) -> Vec<Temp> {
        let mut groups: BTreeMap<&str, (f64, u32)> = BTreeMap::new();
        for (key, size, label) in &self.temps {
            if let Some(v) = self
                .float(*key, *size)
                .filter(|v| plausible_temp(label, *v))
            {
                let e = groups.entry(label.as_str()).or_default();
                e.0 += v;
                e.1 += 1;
            }
        }
        // Fixed order: CPU, GPU, then the rest.
        let order = [
            CPU_P,
            "CPU efficiency cores",
            "GPU",
            "SSD",
            "Battery",
            "Airflow",
            "Wi-Fi",
        ];
        order
            .iter()
            .filter_map(|l| {
                let (sum, n) = groups.get(l)?;
                Some(Temp {
                    label: (*l).to_string(),
                    celsius: sum / f64::from(*n),
                })
            })
            .collect()
    }

    fn read_fans(&self) -> Vec<Fan> {
        self.fans
            .iter()
            .filter_map(|f| {
                let rpm = self.float(f.actual, 4)?;
                Some(Fan {
                    name: f.name.clone(),
                    rpm: rpm.max(0.0),
                    min_rpm: f.min,
                    max_rpm: f.max,
                })
            })
            .collect()
    }
}

/// A power-gated P cluster reads 0-8 C: not a temperature, so the group has
/// no value until it wakes up.
fn plausible_temp(group: &str, v: f64) -> bool {
    let min = if group == CPU_P { 15.0 } else { 5.0 };
    v > min && v < 150.0
}

// ---------- the source ----------

pub struct MacSource {
    start: Instant,
    kinds: Vec<Option<CoreKind>>,
    mem_total: Option<u64>,
    /// `None` until first needed; `Some(None)` if the SMC cannot be opened.
    smc: Option<Option<Smc>>,
}

// The SMC connection is a Mach port number: usable from any thread.
unsafe impl Send for MacSource {}

impl MacSource {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            kinds: core_kinds(),
            mem_total: sysctl_by_name::<u64>(c"hw.memsize"),
            smc: None,
        }
    }
}

impl NowSource for MacSource {
    fn chip(&mut self) -> Option<String> {
        sysctl_string(c"machdep.cpu.brand_string")
    }

    fn read(&mut self) -> RawNow {
        let smc = self.smc.get_or_insert_with(Smc::open).as_ref();
        let vm = self.mem_total.and_then(read_vm);
        RawNow {
            mono_ms: self.start.elapsed().as_millis() as u64,
            cores: read_cores(&self.kinds),
            gpu_percent: read_gpu(),
            swap: read_swap(),
            swap_io: vm.as_ref().map(|v| (v.swap_in, v.swap_out)),
            mem: vm.map(|v| v.mem),
            disk: read_disk(),
            net: read_net(),
            temps: smc.map(Smc::read_temps),
            fans: smc.map(Smc::read_fans),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine's real sources give sane numbers (the always-present ones).
    #[test]
    fn live_readings_are_sane() {
        let mut src = MacSource::new();
        let a = src.read();
        let mem = a.mem.expect("memory");
        assert!(mem.total > 1 << 30 && mem.used > 0 && mem.used <= mem.total);
        assert!(a.cores.is_some_and(|c| !c.is_empty()));
        assert!(a.net.is_some());
        assert!(a.swap.is_some());
    }

    #[test]
    fn sensor_groups() {
        assert_eq!(temp_group("Tp1A"), Some(CPU_P));
        assert_eq!(temp_group("Tp2G"), Some(CPU_P));
        // Rose only a little under load: not P cores.
        assert_eq!(temp_group("Tp0A"), None);
        assert_eq!(temp_group("Tp16"), None);
        assert_eq!(temp_group("Te05"), Some("CPU efficiency cores"));
        assert_eq!(temp_group("Tg0f"), Some("GPU"));
        assert_eq!(temp_group("TaLP"), Some("Airflow"));
        assert_eq!(temp_group("Ta0C"), None);
        assert_eq!(temp_group("TD00"), None);
        // A sleeping P cluster reads near 0: no value, not 5 C.
        assert!(!plausible_temp(CPU_P, 7.4));
        assert!(plausible_temp(CPU_P, 60.0));
        assert!(plausible_temp("GPU", 30.0));
        assert!(!plausible_temp("GPU", 0.0));
    }

    /// `cargo test --release -p wrapped-core --lib sample_cost -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sample_cost() {
        let rss_kb = || -> u64 {
            let out = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        };
        let before = rss_kb();
        let mut sampler = crate::now::NowSampler::new();
        let t = Instant::now();
        sampler.sample();
        println!(
            "first call (SMC key discovery included): {:.2} ms",
            t.elapsed().as_secs_f64() * 1e3
        );
        let mut ms: Vec<f64> = (0..60)
            .map(|_| {
                let t = Instant::now();
                std::hint::black_box(sampler.sample());
                let d = t.elapsed().as_secs_f64() * 1e3;
                std::thread::sleep(std::time::Duration::from_millis(120));
                d
            })
            .collect();
        let time = |name: &str, f: &dyn Fn()| {
            let t = Instant::now();
            for _ in 0..20 {
                f();
            }
            println!("  {name}: {:.2} ms", t.elapsed().as_secs_f64() * 1e3 / 20.0);
        };
        let smc = Smc::open().unwrap();
        println!(
            "  smc keys: {} temps, {} fans",
            smc.temps.len(),
            smc.fans.len()
        );
        time("cores", &|| {
            let _ = read_cores(&[]);
        });
        time("gpu", &|| {
            let _ = read_gpu();
        });
        time("vm", &|| {
            let _ = read_vm(1 << 34);
        });
        time("swap", &|| {
            let _ = read_swap();
        });
        time("disk", &|| {
            let _ = read_disk();
        });
        time("net", &|| {
            let _ = read_net();
        });
        time("temps", &|| {
            let _ = smc.read_temps();
        });
        time("fans", &|| {
            let _ = smc.read_fans();
        });
        ms.sort_by(f64::total_cmp);
        println!("RSS {before} KB -> {} KB (this test process)", rss_kb());
        println!("60 calls: median {:.2} ms, worst {:.2} ms", ms[30], ms[59]);
    }
}
