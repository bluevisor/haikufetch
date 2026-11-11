//! Facts every Unix answers the same way: user, host name, kernel, filesystem space, terminal.

use std::{
    ffi::{CStr, CString},
    mem::MaybeUninit,
};

use crate::info::Usage;

pub fn username() -> Option<String> {
    if let Some(user) = std::env::var("USER").ok().filter(|u| !u.is_empty()) {
        return Some(user);
    }
    // SAFETY: getpwuid returns a pointer into static storage or null; both fields are checked
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_name.is_null() {
            return None;
        }
        Some(CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
    }
}

pub fn hostname() -> Option<String> {
    let mut buffer = [0 as libc::c_char; 256];
    // SAFETY: one byte short of the buffer, so the result is always NUL-terminated
    let ok = unsafe { libc::gethostname(buffer.as_mut_ptr(), buffer.len() - 1) } == 0;
    if !ok {
        return None;
    }
    // SAFETY: NUL-terminated above
    let name = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    (!name.is_empty()).then_some(name)
}

pub struct Uname {
    pub sysname: String,
    pub release: String,
    pub version: String,
}

pub fn uname() -> Option<Uname> {
    let mut raw = MaybeUninit::<libc::utsname>::zeroed();
    // SAFETY: uname fills the struct it's given
    if unsafe { libc::uname(raw.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: initialized by the successful call
    let raw = unsafe { raw.assume_init() };
    // SAFETY: every utsname field is a NUL-terminated array
    let field = |f: &[libc::c_char]| {
        unsafe { CStr::from_ptr(f.as_ptr()) }
            .to_string_lossy()
            .into_owned()
    };
    Some(Uname {
        sysname: field(&raw.sysname),
        release: field(&raw.release),
        version: field(&raw.version),
    })
}

/// (name, release): ("Darwin", "25.6.0"), ("Linux", "6.17.2-arch1-1"). Haiku's release is always
/// "1", its hrev (first word of the version) says which build it is
pub fn kernel() -> Option<(String, String)> {
    let u = uname()?;
    if cfg!(target_os = "haiku") {
        let hrev = u
            .version
            .split_whitespace()
            .next()
            .unwrap_or(&u.release)
            .to_string();
        return Some((u.sysname, hrev));
    }
    Some((u.sysname, u.release))
}

/// Space on the filesystem holding `path`. Free is what a user can still write (no root reserve),
/// the number `df` and fastfetch show
pub fn filesystem_usage(path: &str) -> Option<Usage> {
    let path = CString::new(path).ok()?;
    let (total, free) = filesystem_space(&path)?;
    (total > 0).then(|| Usage {
        used: total.saturating_sub(free),
        total,
    })
}

/// Darwin's statvfs counts blocks in 32 bits and overflows on big disks, statfs doesn't
#[cfg(target_os = "macos")]
fn filesystem_space(path: &CStr) -> Option<(u64, u64)> {
    let mut stat = MaybeUninit::<libc::statfs>::zeroed();
    // SAFETY: statfs fills the struct for a valid C path
    if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: initialized by the successful call
    let stat = unsafe { stat.assume_init() };
    let block = stat.f_bsize as u64;
    Some((
        stat.f_blocks.checked_mul(block)?,
        stat.f_bavail.checked_mul(block)?,
    ))
}

#[cfg(not(target_os = "macos"))]
#[allow(clippy::useless_conversion, clippy::unnecessary_cast)]
fn filesystem_space(path: &CStr) -> Option<(u64, u64)> {
    let mut stat = MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: statvfs fills the struct for a valid C path
    if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: initialized by the successful call
    let stat = unsafe { stat.assume_init() };
    // Field widths differ (u32 on 32-bit Linux, signed on Haiku), so convert each checked
    let block = u64::try_from(stat.f_frsize).ok()?;
    let blocks = u64::try_from(stat.f_blocks).ok()?;
    let available = u64::try_from(stat.f_bavail).ok()?;
    Some((blocks.checked_mul(block)?, available.checked_mul(block)?))
}

/// The terminal emulator, from what terminals tell their children. Falls back to the process
/// tree (the shell's parent) where no variable says
pub fn terminal() -> Option<String> {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());

    if let Some(program) = var("TERM_PROGRAM") {
        let name = match program.as_str() {
            "Apple_Terminal" => "Terminal",
            "iTerm.app" => "iTerm2",
            "vscode" => "VS Code",
            "WezTerm" => "WezTerm",
            "ghostty" => "Ghostty",
            "tmux" => "tmux",
            other => other,
        };
        return Some(match var("TERM_PROGRAM_VERSION") {
            Some(version) if name != "tmux" => format!("{name} {version}"),
            _ => name.to_string(),
        });
    }
    let known = [
        ("KITTY_WINDOW_ID", "kitty"),
        ("ALACRITTY_WINDOW_ID", "Alacritty"),
        ("GHOSTTY_RESOURCES_DIR", "Ghostty"),
        ("WEZTERM_EXECUTABLE", "WezTerm"),
        ("KONSOLE_VERSION", "Konsole"),
        ("WT_SESSION", "Windows Terminal"),
        ("TERMINATOR_UUID", "Terminator"),
    ];
    if let Some((_, name)) = known.iter().find(|(v, _)| var(v).is_some()) {
        return Some(name.to_string());
    }
    if let Some(name) = crate::shell::terminal_process() {
        return Some(name);
    }
    if var("SSH_TTY").is_some() || var("SSH_CONNECTION").is_some() {
        return Some(String::from("SSH"));
    }
    var("TERM").filter(|t| t != "dumb")
}
