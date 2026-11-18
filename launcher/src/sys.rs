// SPDX-License-Identifier: MIT
//! System helpers: paths, x6100.conf, running commands, a few libc calls.

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// A path on the radio. `X6100_ROOT` (tests, a desktop) puts all of them
/// under another directory.
pub fn path(p: &str) -> PathBuf {
    match std::env::var_os("X6100_ROOT") {
        Some(root) => {
            let mut r = PathBuf::from(root);
            r.push(p.trim_start_matches('/'));
            r
        }
        None => PathBuf::from(p),
    }
}

/// A setting from /mnt/x6100.conf, else the shipped defaults, else `def`:
/// the same rules as `x6100_cfg` in /usr/libexec/x6100/functions (last
/// KEY=value line wins, quotes and `# comments` dropped; a key present
/// but empty in the user's file means `def`, not the shipped value).
pub fn cfg(key: &str, def: &str) -> String {
    for f in ["/mnt/x6100.conf", "/usr/share/x6100/x6100.conf"] {
        let Ok(text) = std::fs::read_to_string(path(f)) else {
            continue;
        };
        if let Some(v) = cfg_lookup(&text, key) {
            return if v.is_empty() { def.to_string() } else { v };
        }
    }
    def.to_string()
}

pub fn cfg_on(key: &str, def: &str) -> bool {
    matches!(
        cfg(key, def).to_ascii_lowercase().as_str(),
        "on" | "yes" | "true" | "1"
    )
}

fn cfg_lookup(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let l = line.trim_start();
        let Some(rest) = l.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(v) = rest.strip_prefix('=') else {
            continue;
        };
        let mut v = v.trim().trim_end_matches('\r').to_string();
        if let Some(i) = v.find('#') {
            v.truncate(i);
        }
        let v = v.trim_end();
        let v = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
            .unwrap_or(v);
        found = Some(v.to_string());
    }
    found
}

/// Run a command (no shell), up to `timeout`. Ok(stdout) when it exits
/// with 0, else Err(the most useful line of what it said).
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{cmd}: {e}"))?;
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let t_out = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let t_err = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(30)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = t_out.join().unwrap_or_default();
    let err = t_err.join().unwrap_or_default();
    match status {
        Some(st) if st.success() => Ok(out),
        Some(st) => {
            let msg = last_line(&err).or_else(|| last_line(&out));
            Err(msg.unwrap_or_else(|| format!("{cmd} failed ({st})")))
        }
        None => Err(format!("{cmd}: no answer in {} s", timeout.as_secs())),
    }
}

fn last_line(s: &str) -> Option<String> {
    s.lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .map(|l| l.strip_prefix("Error: ").unwrap_or(l).to_string())
}

/// Split one line of `nmcli -t` / `x6100-bt` output: fields separated by
/// ':', with ':' and '\' inside fields escaped by '\'.
pub fn split_terse(line: &str) -> Vec<String> {
    let mut fields = vec![String::new()];
    let mut esc = false;
    for c in line.chars() {
        if esc {
            fields.last_mut().unwrap().push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            fields.push(String::new());
        } else {
            fields.last_mut().unwrap().push(c);
        }
    }
    fields
}

/// IPv4 addresses of the interfaces that are up (not loopback).
pub fn ipv4_addresses() -> Vec<(String, String)> {
    let Ok(out) = run("ip", &["-o", "-4", "addr", "show", "scope", "global"], Duration::from_secs(3))
    else {
        return Vec::new();
    };
    out.lines()
        .filter_map(|l| {
            // "3: wlan0    inet 192.0.2.10/24 brd ... scope global wlan0"
            let mut w = l.split_whitespace();
            let dev = w.nth(1)?.to_string();
            let mut w = w.skip_while(|x| *x != "inet");
            w.next()?;
            let ip = w.next()?.split('/').next()?.to_string();
            Some((dev, ip))
        })
        .collect()
}

/// Free bytes on the filesystem holding `p` (`df -P -k`).
pub fn free_bytes(p: &std::path::Path) -> Option<u64> {
    let out = run("df", &["-P", "-k", &p.to_string_lossy()], Duration::from_secs(3)).ok()?;
    let line = out.lines().nth(1)?;
    let kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

pub fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "x6100".to_string())
}

/// Lock, also after another thread panicked while holding the lock.
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[allow(non_camel_case_types)]
type c_int = i32;

extern "C" {
    fn kill(pid: c_int, sig: c_int) -> c_int;
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn ioctl(fd: c_int, req: std::os::raw::c_ulong, ...) -> c_int;
}

pub const SIGKILL: i32 = 9;
pub const SIGTERM: i32 = 15;

/// Send `sig` to the process group `pgid`.
pub fn kill_group(pgid: u32, sig: i32) {
    // SAFETY: kill(2) with a negative pid signals that process group.
    unsafe {
        kill(-(pgid as c_int), sig);
    }
}

/// The virtual terminal in graphics mode, so the framebuffer console
/// never draws over the screen.
pub fn console_graphics() {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        const KDSETMODE: std::os::raw::c_ulong = 0x4b3a;
        const KD_GRAPHICS: c_int = 1;
        for tty in ["/dev/tty1", "/dev/tty0"] {
            if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(tty) {
                // SAFETY: KDSETMODE takes an int argument; the fd is open.
                unsafe {
                    ioctl(f.as_raw_fd(), KDSETMODE, KD_GRAPHICS);
                }
                return;
            }
        }
    }
}

/// xres, yres, bits per pixel and yoffset of a framebuffer device.
pub fn fb_var_info(f: &std::fs::File) -> Option<(usize, usize, usize, usize)> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        const FBIOGET_VSCREENINFO: std::os::raw::c_ulong = 0x4600;
        // struct fb_var_screeninfo: 40 32-bit words; xres, yres,
        // xres_virtual, yres_virtual, xoffset, yoffset, bits_per_pixel, ...
        let mut v = [0u32; 40];
        // SAFETY: the buffer is as large as the kernel's structure.
        let r = unsafe { ioctl(f.as_raw_fd(), FBIOGET_VSCREENINFO, v.as_mut_ptr()) };
        if r == 0 {
            return Some((v[0] as usize, v[1] as usize, v[6] as usize, v[5] as usize));
        }
    }
    let _ = f;
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_lines() {
        let t = "# c\nA=1\n B = \"two words\" # note\nC=\nA=3\nD='x'\r\nAB=no\n";
        assert_eq!(cfg_lookup(t, "A").as_deref(), Some("3"));
        assert_eq!(cfg_lookup(t, "B").as_deref(), Some("two words"));
        assert_eq!(cfg_lookup(t, "C").as_deref(), Some(""));
        assert_eq!(cfg_lookup(t, "D").as_deref(), Some("x"));
        assert_eq!(cfg_lookup(t, "E"), None);
    }

    #[test]
    fn terse_fields() {
        assert_eq!(
            split_terse(r"00\:11\:22:yes:My\\Net\:5G"),
            vec!["00:11:22", "yes", r"My\Net:5G"]
        );
        assert_eq!(split_terse(""), vec![""]);
    }
}
