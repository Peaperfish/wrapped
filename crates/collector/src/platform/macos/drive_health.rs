use std::io::{ErrorKind, Read};
use std::os::fd::OwnedFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use wrapped_core::collector::Sink;
use wrapped_core::collectors::drive_health::{ProbeResult, Unavailable};

const TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(25);
const OUTPUT_LIMIT: usize = 256 * 1024;

fn smartctl() -> Option<PathBuf> {
    // Finder and launchd may provide only the system PATH.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let standard = [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/local/bin",
        "/opt/local/sbin",
    ];
    std::env::split_paths(&path)
        .chain(standard.into_iter().map(PathBuf::from))
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join("smartctl"))
        .find(|candidate| {
            candidate
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
}

pub fn read(out: &Sink) -> ProbeResult {
    let helper = smartctl().ok_or(Unavailable::HelperMissing)?;
    let device = super::counters::internal_nvme_device().ok_or(Unavailable::NoInternalNvme)?;
    let (mut reader, writer) = UnixStream::pair().map_err(|_| Unavailable::HealthData)?;
    reader
        .set_nonblocking(true)
        .map_err(|_| Unavailable::HealthData)?;

    let mut child = {
        let mut cmd = Command::new(helper);
        cmd.args(["-H", "-A", "-j", "-d", "nvme"])
            .arg(device)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(writer)))
            .stderr(Stdio::null());
        out.configure_command(&mut cmd);
        cmd.spawn().map_err(|e| match e.kind() {
            ErrorKind::NotFound => Unavailable::HelperMissing,
            _ => Unavailable::HealthData,
        })?
    };
    let result = capture(&mut child, &mut reader, out);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

fn capture(child: &mut Child, reader: &mut UnixStream, out: &Sink) -> ProbeResult {
    let deadline = Instant::now() + TIMEOUT;
    let mut stdout = Vec::new();
    let mut chunk = [0; 4096];
    let mut eof = false;
    while Instant::now() < deadline {
        if out.cancelled() {
            return Err(Unavailable::HealthData);
        }
        if eof {
            if let Some(status) = child.try_wait().map_err(|_| Unavailable::HealthData)? {
                // smartctl's exit code is a bitmask. A health warning or a
                // partially failed SMART command can still include valid data.
                return match status.code() {
                    Some(code) if code & 0b11 == 0 => Ok(stdout),
                    _ => Err(Unavailable::HealthData),
                };
            }
        } else {
            match reader.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(n) if stdout.len() + n <= OUTPUT_LIMIT => {
                    stdout.extend_from_slice(&chunk[..n]);
                    continue;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                _ => return Err(Unavailable::HealthData),
            }
        }
        std::thread::sleep(POLL);
    }
    Err(Unavailable::TimedOut)
}
