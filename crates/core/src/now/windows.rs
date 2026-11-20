//! PDH's English counter paths also work on localized installations.
use super::*;
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    NetworkManagement::IpHelper::*,
    System::{Performance::*, SystemInformation::*},
};

struct Query(PDH_HQUERY);
// PDH queries can be used from different threads. NowSampler gives exclusive access.
unsafe impl Send for Query {}
impl Drop for Query {
    fn drop(&mut self) {
        unsafe {
            PdhCloseQuery(self.0);
        }
    }
}
impl Query {
    fn open() -> Option<Self> {
        let mut h = null_mut();
        (unsafe { PdhOpenQueryW(null(), 0, &mut h) } == 0).then(|| Self(h))
    }
    fn add(&self, path: &str) -> PDH_HCOUNTER {
        let path = path.encode_utf16().chain([0]).collect::<Vec<_>>();
        let mut h = null_mut();
        unsafe {
            PdhAddEnglishCounterW(self.0, path.as_ptr(), 0, &mut h);
        }
        h
    }
    fn collect(&self) -> bool {
        unsafe { PdhCollectQueryData(self.0) == 0 }
    }
}
fn raw(h: PDH_HCOUNTER) -> Option<(u64, u64)> {
    let mut r = PDH_RAW_COUNTER::default();
    if h.is_null() || unsafe { PdhGetRawCounterValue(h, null_mut(), &mut r) } != 0 || r.CStatus > 1
    {
        return None;
    }
    Some((
        r.FirstValue.try_into().ok()?,
        r.SecondValue.try_into().ok()?,
    ))
}

struct Counters {
    query: Query,
    read: PDH_HCOUNTER,
    write: PDH_HCOUNTER,
    cores: Vec<PDH_HCOUNTER>,
    gpu: PDH_HCOUNTER,
}
// Handles are owned by query and only accessed through an exclusive sampler.
unsafe impl Send for Counters {}
impl Counters {
    fn new(full: bool) -> Option<Self> {
        let query = Query::open()?;
        let read = query.add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec");
        let write = query.add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec");
        let mut cores = vec![];
        let mut gpu = null_mut();
        if full {
            let mut info = SYSTEM_INFO::default();
            unsafe {
                GetSystemInfo(&mut info);
            }
            for i in 0..info.dwNumberOfProcessors {
                cores.push(query.add(&format!(r"\Processor({i})\% Processor Time")));
            }
            gpu = query.add(r"\GPU Engine(*)\Utilization Percentage");
        }
        Some(Self {
            query,
            read,
            write,
            cores,
            gpu,
        })
    }
    fn disk(&self) -> Option<(u64, u64)> {
        Some((raw(self.read)?.0, raw(self.write)?.0))
    }
}
pub fn disk_totals() -> Option<(u64, u64)> {
    let c = Counters::new(false)?;
    if !c.query.collect() {
        return None;
    }
    c.disk()
}
fn gpu_load(h: PDH_HCOUNTER) -> Option<f64> {
    if h.is_null() {
        return None;
    }
    // Sum process contributions for each physical engine, then report the busiest.
    unsafe {
        let (mut bytes, mut count) = (0, 0);
        if PdhGetFormattedCounterArrayW(h, PDH_FMT_DOUBLE, &mut bytes, &mut count, null_mut())
            != PDH_MORE_DATA
            || bytes > 16 * 1024 * 1024
        {
            return None;
        }
        let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        let items = buffer.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        if PdhGetFormattedCounterArrayW(h, PDH_FMT_DOUBLE, &mut bytes, &mut count, items) != 0 {
            return None;
        }
        let mut engines = HashMap::new();
        for item in std::slice::from_raw_parts(items, count as usize) {
            if item.FmtValue.CStatus > 1 || item.szName.is_null() {
                continue;
            }
            let mut len = 0;
            while *item.szName.add(len) != 0 {
                len += 1;
            }
            let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.szName, len));
            if let Some((_, engine)) = name.split_once("_luid_") {
                *engines.entry(engine.to_owned()).or_insert(0.) +=
                    item.FmtValue.Anonymous.doubleValue;
            }
        }
        engines.into_values().reduce(f64::max)
    }
}

fn network() -> Option<Vec<RawIface>> {
    unsafe {
        let mut table = null_mut();
        if GetIfTable2(&mut table) != 0 {
            return None;
        }
        let rows =
            std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let interfaces = rows
            .iter()
            .filter(|r| {
                r.Type != IF_TYPE_SOFTWARE_LOOPBACK
                    && r.InterfaceAndOperStatusFlags._bitfield & 1 != 0
            })
            .map(|r| RawIface {
                id: r.InterfaceIndex,
                rx: r.InOctets,
                tx: r.OutOctets,
            })
            .collect();
        FreeMibTable(table.cast());
        Some(interfaces)
    }
}
pub(super) struct WindowsSource {
    counters: Option<Counters>,
}
impl WindowsSource {
    pub fn new() -> Self {
        Self {
            counters: Counters::new(true),
        }
    }
}
impl NowSource for WindowsSource {
    fn chip(&mut self) -> Option<String> {
        std::env::var("PROCESSOR_IDENTIFIER").ok()
    }
    fn read(&mut self) -> RawNow {
        let mut m = MEMORYSTATUSEX {
            dwLength: size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        let mem = (unsafe { GlobalMemoryStatusEx(&mut m) } != 0).then(|| RawMem {
            used: m.ullTotalPhys.saturating_sub(m.ullAvailPhys),
            total: m.ullTotalPhys,
            pressure: None,
            compressed: None,
        });
        let mut s = RawNow {
            mono_ms: unsafe { GetTickCount64() },
            mem,
            net: network(),
            ..Default::default()
        };
        if let Some(c) = &self.counters
            && c.query.collect()
        {
            s.disk = c.disk();
            s.gpu_percent = gpu_load(c.gpu);
            s.cores = c
                .cores
                .iter()
                .map(|&h| {
                    let (idle, total) = raw(h)?;
                    Some(RawCore {
                        busy: (total.checked_sub(idle)? / 10000) as u32,
                        idle: (idle / 10000) as u32,
                        kind: None,
                    })
                })
                .collect();
        }
        s
    }
}
