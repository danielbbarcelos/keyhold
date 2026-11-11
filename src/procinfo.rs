//! What the kernel says about a process: its name, parent, owner, age, arguments and program file.
//!
//! Everything here is read from the kernel, never from the process itself, because the answers
//! decide who is asking for a secret (the agent's peer and its parent) and which process a "while
//! it is running" permission belongs to. Linux keeps all of it in `/proc`; macOS has no `/proc`, so
//! there it comes from `proc_pidinfo`, `proc_pidpath` and the `KERN_PROCARGS2` sysctl.
//!
//! Every function answers `None` (or an empty list) when the kernel will not say, and the callers
//! treat that as "unknown", never as "fine".

use std::path::PathBuf;

/// The short command name (`comm`): "keyhold", "claude", "node".
pub fn name(pid: i32) -> Option<String> {
    imp::name(pid).map(|n| n.trim().to_string()).filter(|n| !n.is_empty())
}

/// The parent's pid.
pub fn parent(pid: i32) -> Option<i32> {
    imp::parent(pid)
}

/// The user id that owns the process.
pub fn owner(pid: i32) -> Option<u32> {
    imp::owner(pid)
}

/// When the process started, as an opaque number that is the same for as long as the process lives
/// and different for whatever later reuses its pid. Only ever compared for equality.
pub fn start_mark(pid: i32) -> Option<u64> {
    imp::start_mark(pid)
}

/// Seconds since the process started.
pub fn age_seconds(pid: i32) -> Option<u64> {
    imp::age_seconds(pid)
}

/// The command line, program first.
pub fn args(pid: i32) -> Option<Vec<String>> {
    imp::args(pid)
}

/// The program file the process runs.
pub fn exe(pid: i32) -> Option<PathBuf> {
    imp::exe(pid)
}

/// Every pid on the machine the kernel will tell this user about.
pub fn all_pids() -> Vec<i32> {
    imp::all_pids()
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::path::PathBuf;

    /// The fields of `/proc/<pid>/stat` after the command name, which sits in parentheses and may
    /// itself contain spaces and parentheses: the parse starts after the last `)`. Index 0 here is
    /// field 3 of the file (the state).
    fn stat_fields(pid: i32) -> Option<Vec<String>> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = stat.rfind(')')?;

        Some(stat[after + 1..].split_whitespace().map(str::to_string).collect())
    }

    pub fn name(pid: i32) -> Option<String> {
        fs::read_to_string(format!("/proc/{pid}/comm")).ok()
    }

    pub fn parent(pid: i32) -> Option<i32> {
        stat_fields(pid)?.get(1)?.parse().ok()
    }

    pub fn owner(pid: i32) -> Option<u32> {
        fs::metadata(format!("/proc/{pid}")).ok().map(|meta| meta.uid())
    }

    /// Field 22: clock ticks since boot.
    pub fn start_mark(pid: i32) -> Option<u64> {
        stat_fields(pid)?.get(19)?.parse().ok()
    }

    pub fn age_seconds(pid: i32) -> Option<u64> {
        let uptime: f64 = fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
        // SAFETY: `sysconf` reads a constant of the running kernel.
        let ticks = match unsafe { libc::sysconf(libc::_SC_CLK_TCK) } {
            t if t > 0 => t as f64,
            _ => 100.0,
        };

        Some((uptime - start_mark(pid)? as f64 / ticks).max(0.0) as u64)
    }

    pub fn args(pid: i32) -> Option<Vec<String>> {
        let raw = fs::read(format!("/proc/{pid}/cmdline")).ok()?;

        Some(
            raw.split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect(),
        )
    }

    pub fn exe(pid: i32) -> Option<PathBuf> {
        // The link itself, not its target: `/proc/<pid>/exe` still opens the file a process runs
        // after a reinstall has replaced the one at that path.
        let link = PathBuf::from(format!("/proc/{pid}/exe"));
        fs::metadata(&link).ok().map(|_| link)
    }

    pub fn all_pids() -> Vec<i32> {
        fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::{c_char, c_int, c_void};
    use std::path::PathBuf;

    fn bsd_info(pid: i32) -> Option<libc::proc_bsdinfo> {
        // SAFETY: `proc_bsdinfo` is plain data, and `proc_pidinfo` writes at most `size` bytes into
        // it; the answer counts only when it wrote the whole struct.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as c_int;
            let wrote = libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, &mut info as *mut _ as *mut c_void, size);

            (wrote == size).then_some(info)
        }
    }

    fn text(raw: &[c_char]) -> Option<String> {
        let bytes: Vec<u8> = raw.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();

        String::from_utf8(bytes).ok()
    }

    pub fn name(pid: i32) -> Option<String> {
        let info = bsd_info(pid)?;

        // `pbi_name` is the longer one; `pbi_comm` is cut at 16 bytes. Either can be empty.
        text(&info.pbi_name).filter(|n| !n.is_empty()).or_else(|| text(&info.pbi_comm))
    }

    pub fn parent(pid: i32) -> Option<i32> {
        bsd_info(pid).map(|info| info.pbi_ppid as i32)
    }

    pub fn owner(pid: i32) -> Option<u32> {
        bsd_info(pid).map(|info| info.pbi_uid)
    }

    /// Microseconds since the epoch.
    pub fn start_mark(pid: i32) -> Option<u64> {
        bsd_info(pid).map(|info| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }

    pub fn age_seconds(pid: i32) -> Option<u64> {
        let started = bsd_info(pid)?.pbi_start_tvsec;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();

        Some(now.saturating_sub(started))
    }

    /// `KERN_PROCARGS2`: argc, then the executable path, padding, then argv, each NUL-terminated.
    pub fn args(pid: i32) -> Option<Vec<String>> {
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        let mut size: libc::size_t = 0;

        // SAFETY: the first call only asks for the size; the second writes at most `size` bytes.
        unsafe {
            if libc::sysctl(mib.as_mut_ptr(), 3, std::ptr::null_mut(), &mut size, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
        }

        let mut buffer = vec![0u8; size];
        // SAFETY: as above.
        unsafe {
            if libc::sysctl(mib.as_mut_ptr(), 3, buffer.as_mut_ptr() as *mut c_void, &mut size, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
        }
        buffer.truncate(size);

        let argc = i32::from_ne_bytes(buffer.get(..4)?.try_into().ok()?) as usize;
        let rest = &buffer[4..];
        // Skip the executable path and the NULs that pad it.
        let start = rest.iter().position(|b| *b == 0)?;
        let rest = &rest[start..];
        let start = rest.iter().position(|b| *b != 0)?;

        Some(
            rest[start..]
                .split(|b| *b == 0)
                .take(argc)
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect(),
        )
    }

    pub fn exe(pid: i32) -> Option<PathBuf> {
        let mut buffer = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: `proc_pidpath` writes at most the buffer's size, NUL-terminated.
        let wrote = unsafe { libc::proc_pidpath(pid, buffer.as_mut_ptr() as *mut c_void, buffer.len() as u32) };

        if wrote <= 0 {
            return None;
        }

        let path = CStr::from_bytes_until_nul(&buffer).ok()?.to_str().ok()?;
        Some(PathBuf::from(path))
    }

    pub fn all_pids() -> Vec<i32> {
        // SAFETY: a null buffer asks for the count; the second call fills at most the buffer.
        unsafe {
            let count = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if count <= 0 {
                return Vec::new();
            }

            let mut pids = vec![0 as c_int; count as usize + 32];
            let bytes = (pids.len() * std::mem::size_of::<c_int>()) as c_int;
            let found = libc::proc_listallpids(pids.as_mut_ptr() as *mut c_void, bytes);

            pids.truncate(found.max(0) as usize);
            pids.into_iter().filter(|pid| *pid > 0).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_described_by_the_kernel() {
        let me = std::process::id() as i32;

        assert!(name(me).is_some_and(|n| !n.is_empty()));
        assert!(parent(me).is_some_and(|p| p > 0));
        // SAFETY: `getuid` reads a field of this process and cannot fail.
        assert_eq!(owner(me), Some(unsafe { libc::getuid() }));
        assert!(start_mark(me).is_some());
        assert_eq!(start_mark(me), start_mark(me), "the mark is stable");
        assert!(age_seconds(me).is_some());
        assert!(args(me).is_some_and(|a| !a.is_empty()));
        assert!(exe(me).is_some());
        assert!(all_pids().contains(&me));
    }

    #[test]
    fn a_reaped_process_is_described_by_nothing() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id() as i32;
        child.wait().unwrap();

        assert!(name(pid).is_none());
        assert!(start_mark(pid).is_none());
    }
}
