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

/// Whether the process runs as the same user as this one. Unknown is no.
pub fn is_mine(pid: i32) -> bool {
    imp::is_mine(pid)
}

#[cfg(unix)]
mod mine {
    pub fn is_mine(pid: i32) -> bool {
        // SAFETY: `getuid` reads a field of this process and cannot fail.
        super::owner(pid) == Some(unsafe { libc::getuid() })
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub use super::mine::is_mine;
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
    pub use super::mine::is_mine;
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

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, UNICODE_STRING};
    use windows_sys::Win32::Security::{EqualSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// A handle that closes itself.
    struct Owned(HANDLE);

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful open and is closed exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: i32) -> Option<Owned> {
        // SAFETY: plain call; a null handle is the failure.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };

        (!handle.is_null()).then(|| Owned(handle))
    }

    /// Every process, from a ToolHelp snapshot: (pid, parent, executable name).
    fn snapshot() -> Vec<(i32, i32, String)> {
        let mut out = Vec::new();

        // SAFETY: the entry is plain data with its size set, as the API requires.
        unsafe {
            let shot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if shot == INVALID_HANDLE_VALUE || shot.is_null() {
                return out;
            }
            let shot = Owned(shot);

            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            let mut more = Process32FirstW(shot.0, &mut entry) != 0;
            while more {
                let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                out.push((entry.th32ProcessID as i32, entry.th32ParentProcessID as i32, name));
                more = Process32NextW(shot.0, &mut entry) != 0;
            }
        }

        out
    }

    fn entry(pid: i32) -> Option<(i32, i32, String)> {
        snapshot().into_iter().find(|(p, _, _)| *p == pid)
    }

    /// "keyhold.exe" reads as "keyhold", the name the other systems give. A process that has
    /// exited but is still held open by someone stays in the snapshot; it is not running.
    pub fn name(pid: i32) -> Option<String> {
        start_mark(pid)?;
        let (_, _, name) = entry(pid)?;

        Some(name.strip_suffix(".exe").or_else(|| name.strip_suffix(".EXE")).unwrap_or(&name).to_string())
    }

    pub fn parent(pid: i32) -> Option<i32> {
        start_mark(pid)?;
        entry(pid).map(|(_, parent, _)| parent)
    }

    /// Windows has no uid; "owner" is answered by [`is_mine`] instead.
    pub fn owner(_pid: i32) -> Option<u32> {
        None
    }

    fn filetime(time: FILETIME) -> u64 {
        ((time.dwHighDateTime as u64) << 32) | time.dwLowDateTime as u64
    }

    /// The creation time, in 100 ns units since 1601.
    pub fn start_mark(pid: i32) -> Option<u64> {
        let process = open(pid)?;
        let zero = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);

        // SAFETY: four out-parameters of the right type.
        let ok = unsafe { GetProcessTimes(process.0, &mut created, &mut exited, &mut kernel, &mut user) } != 0;

        // A process that has exited but whose handle lingers is not running.
        (ok && filetime(exited) == 0).then(|| filetime(created))
    }

    pub fn age_seconds(pid: i32) -> Option<u64> {
        let started = start_mark(pid)?;
        let mut now = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        // SAFETY: one out-parameter.
        unsafe { GetSystemTimeAsFileTime(&mut now) };

        Some(filetime(now).saturating_sub(started) / 10_000_000)
    }

    /// The command line, read with `NtQueryInformationProcess` (class 60,
    /// ProcessCommandLineInformation), split the way the C runtime splits it.
    pub fn args(pid: i32) -> Option<Vec<String>> {
        let process = open(pid)?;
        let mut buffer = vec![0u8; 64 * 1024];
        let mut needed = 0u32;

        // SAFETY: the buffer is large and aligned enough for the UNICODE_STRING the call puts at its
        // start, with the text right after it, inside the same buffer.
        let line = unsafe {
            let status = windows_sys::Wdk::System::Threading::NtQueryInformationProcess(
                process.0,
                60,
                buffer.as_mut_ptr() as *mut _,
                buffer.len() as u32,
                &mut needed,
            );
            if status < 0 {
                return None;
            }

            let text = &*(buffer.as_ptr() as *const UNICODE_STRING);
            if text.Buffer.is_null() {
                return None;
            }
            let chars = std::slice::from_raw_parts(text.Buffer, text.Length as usize / 2);
            String::from_utf16_lossy(chars)
        };

        Some(split_command_line(&line))
    }

    /// Spaces separate, double quotes group, a backslash before a quote escapes it.
    pub(super) fn split_command_line(line: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        let mut any = false;
        let mut chars = line.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&'"') => {
                    current.push('"');
                    chars.next();
                    any = true;
                }
                '"' => {
                    quoted = !quoted;
                    any = true;
                }
                ' ' | '\t' if !quoted => {
                    if any {
                        out.push(std::mem::take(&mut current));
                        any = false;
                    }
                }
                other => {
                    current.push(other);
                    any = true;
                }
            }
        }
        if any {
            out.push(current);
        }

        out
    }

    pub fn exe(pid: i32) -> Option<PathBuf> {
        let process = open(pid)?;
        let mut buffer = vec![0u16; 32 * 1024];
        let mut size = buffer.len() as u32;

        // SAFETY: the size says how much the buffer holds; the call says how much it wrote.
        let ok = unsafe { QueryFullProcessImageNameW(process.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size) } != 0;

        ok.then(|| PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize])))
    }

    pub fn all_pids() -> Vec<i32> {
        snapshot().into_iter().map(|(pid, _, _)| pid).filter(|pid| *pid > 0).collect()
    }

    /// The user a process's token belongs to, as the raw TOKEN_USER buffer.
    fn token_user(process: HANDLE) -> Option<Vec<u8>> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: one out-parameter; the token is closed by `Owned`.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
            return None;
        }
        let token = Owned(token);

        let mut needed = 0u32;
        // SAFETY: the first call asks for the size, the second fills a buffer of that size.
        unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        let mut buffer = vec![0u8; needed as usize];
        let ok = unsafe { GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr() as *mut _, needed, &mut needed) } != 0;

        ok.then_some(buffer)
    }

    pub fn is_mine(pid: i32) -> bool {
        let Some(process) = open(pid) else { return false };
        // SAFETY: the pseudo-handle of this process needs no closing.
        let (Some(theirs), Some(ours)) = (token_user(process.0), token_user(unsafe { GetCurrentProcess() })) else {
            return false;
        };

        // SAFETY: both buffers hold a TOKEN_USER whose SID points inside the same buffer.
        unsafe {
            let theirs = &*(theirs.as_ptr() as *const TOKEN_USER);
            let ours = &*(ours.as_ptr() as *const TOKEN_USER);
            EqualSid(theirs.User.Sid, ours.User.Sid) != 0
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
        assert!(is_mine(me));
        assert!(start_mark(me).is_some());
        assert_eq!(start_mark(me), start_mark(me), "the mark is stable");
        assert!(age_seconds(me).is_some());
        assert!(args(me).is_some_and(|a| !a.is_empty()));
        assert!(exe(me).is_some());
        assert!(all_pids().contains(&me));
    }

    #[cfg(windows)]
    #[test]
    fn a_windows_command_line_splits_like_the_c_runtime() {
        assert_eq!(
            imp::split_command_line(r#""C:\Program Files\keyhold.exe" mcp --x "a b""#),
            ["C:\\Program Files\\keyhold.exe", "mcp", "--x", "a b"]
        );
    }

    #[test]
    fn a_reaped_process_is_described_by_nothing() {
        // The child is dropped before asking: on Windows a handle still open keeps the exited
        // process listed.
        let pid = {
            let mut child = if cfg!(windows) {
                std::process::Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap()
            } else {
                std::process::Command::new("true").spawn().unwrap()
            };
            let pid = child.id() as i32;
            child.wait().unwrap();
            pid
        };

        assert!(name(pid).is_none());
        assert!(start_mark(pid).is_none());
    }
}
